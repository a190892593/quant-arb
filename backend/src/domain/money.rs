//! 金额与数量的精确计算辅助。
//!
//! 约定：
//! - 所有 USDC 金额保留 2–6 位小数；所有资产数量保留最多 18 位，落库前按 8 位截断。
//! - 费率统一使用 bps（万分之一，10_000 = 100%）。
//! - 向下取整用于「可成交数量」，向上取整用于「成本与应还金额」，避免账面虚假盈利。

use rust_decimal::Decimal;
use std::str::FromStr;

pub const BPS_DENOM: i64 = 10_000;

/// 10^decimals（用于人类单位与链上原始单位的换算）。
pub fn unit_scale(decimals: u32) -> Decimal {
    let mut s = Decimal::ONE;
    for _ in 0..decimals.min(36) {
        s *= Decimal::from(10);
    }
    s
}

/// 解析十进制字面量，非法输入直接 panic —— 仅用于常量与测试。
pub fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|_| panic!("invalid decimal literal: {s}"))
}

/// 安全解析：失败返回 None。
pub fn try_dec(s: &str) -> Option<Decimal> {
    Decimal::from_str(s).ok()
}

/// 按 bps 计算比例金额：`v * bps / 10000`。
pub fn bps_of(v: Decimal, bps: i64) -> Decimal {
    v * Decimal::from(bps) / Decimal::from(BPS_DENOM)
}

/// 按 Decimal 形式的 bps 计算（费率来自交易所规则表时使用）。
pub fn bps_of_dec(v: Decimal, bps: Decimal) -> Decimal {
    v * bps / Decimal::from(BPS_DENOM)
}

/// 向下取整到指定步长（数量精度、价格 tick）。
pub fn round_step_down(v: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return v;
    }
    (v / step).floor() * step
}

/// 四舍五入到指定步长。
pub fn round_step_nearest(v: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return v;
    }
    (v / step).round() * step
}

/// 保留 8 位小数（资产数量落库精度）。
pub fn q8(v: Decimal) -> Decimal {
    v.round_dp(8)
}

/// 保留 2 位小数（USDC 金额展示精度）。
pub fn q2(v: Decimal) -> Decimal {
    v.round_dp(2)
}

/// 保留 6 位小数（成本明细精度）。
pub fn q6(v: Decimal) -> Decimal {
    v.round_dp(6)
}

/// 百分比：`part / whole * 100`，whole 为 0 时返回 0。
pub fn pct(part: Decimal, whole: Decimal) -> Decimal {
    if whole.is_zero() {
        Decimal::ZERO
    } else {
        part / whole * Decimal::from(100)
    }
}

/// 相对 bps 差：`(a - b) / b * 10000`。
pub fn diff_bps(a: Decimal, b: Decimal) -> Decimal {
    if b.is_zero() {
        Decimal::ZERO
    } else {
        (a - b) / b * Decimal::from(BPS_DENOM)
    }
}

/// 带符号的 USDC 金额展示，例如 `+12.34` / `-0.87`。
pub fn signed_usd(v: Decimal) -> String {
    let v = q2(v);
    if v > Decimal::ZERO {
        format!("+{v}")
    } else if v < Decimal::ZERO {
        format!("{v}")
    } else {
        "0.00".to_string()
    }
}

/// 无符号 USDC 金额展示。
pub fn usd(v: Decimal) -> String {
    format!("{}", q2(v))
}

/// 资产数量展示：按精度裁剪并去掉无意义的尾随 0。
pub fn qty(v: Decimal) -> String {
    let s = format!("{}", v.round_dp(8));
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// 价格展示：最多 6 位小数。
pub fn price(v: Decimal) -> String {
    let s = format!("{}", v.round_dp(6));
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// 安全除法：分母为 0 返回 0。
pub fn div(a: Decimal, b: Decimal) -> Decimal {
    if b.is_zero() { Decimal::ZERO } else { a / b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bps_math() {
        assert_eq!(bps_of(dec("1000"), 30), dec("3.0"));
        assert_eq!(bps_of(dec("5000"), 10), dec("5.0"));
    }

    #[test]
    fn step_rounding_is_conservative() {
        assert_eq!(round_step_down(dec("1.2345"), dec("0.01")), dec("1.23"));
        assert_eq!(round_step_down(dec("1.2399"), dec("0.001")), dec("1.239"));
        // 不足一个步长时归零，而不是进位放大数量
        assert_eq!(round_step_down(dec("0.004"), dec("0.01")), Decimal::ZERO);
    }

    #[test]
    fn formatting() {
        assert_eq!(signed_usd(dec("12.344")), "+12.34");
        assert_eq!(signed_usd(dec("-0.871")), "-0.87");
        assert_eq!(qty(dec("0.12340000")), "0.1234");
        assert_eq!(price(dec("2793.100000")), "2793.1");
    }

    #[test]
    fn ratios() {
        assert_eq!(diff_bps(dec("100.1"), dec("100.0")), dec("10"));
        assert_eq!(pct(dec("25"), dec("5000")), dec("0.5"));
        assert_eq!(div(dec("1"), Decimal::ZERO), Decimal::ZERO);
    }

    #[test]
    fn decimal_json_is_string() {
        let j = serde_json::to_string(&dec("2793.10")).unwrap();
        assert_eq!(j, "\"2793.10\"", "Decimal 必须以字符串传输，避免浮点误差");
        let v: Decimal = serde_json::from_str("\"2793.10\"").unwrap();
        assert_eq!(v, dec("2793.10"));
    }
}
