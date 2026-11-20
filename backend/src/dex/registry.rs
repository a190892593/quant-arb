//! Token / Pool 准入注册表（PRD §3.3、§8）。
//!
//! 准入是扫描的前置条件，不是事后过滤：未审核或已拒绝的 Token 不进入交易图，
//! 不可信的池不贡献任何边。这样搜索空间不会被污染，也不会把风险池当作套利机会。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::domain::dex_scan::{PoolAdmission, TokenAdmission};
use crate::domain::market::{PoolKind, RiskViolation, Token};
use crate::domain::types::{RiskSeverity, TokenStatus};
use crate::dex::pool::PoolRuntime;

/// 会被判定为 RESTRICTED 的风险标签（PRD §8.2）。
pub const HARD_RISK_TAGS: &[&str] = &[
    "transfer_tax",
    "fee_on_transfer",
    "blacklist",
    "pausable",
    "rebase",
    "proxy_upgradeable",
    "non_standard_erc20",
    "trading_limit",
    "honeypot",
];

/// 池准入阈值。
#[derive(Clone, Debug)]
pub struct PoolLimits {
    /// 单池最低 TVL（USDC）。低于阈值的池不进入图。
    pub min_tvl_usd: Decimal,
    /// 池状态最大年龄（毫秒）。超过即视为不可报价。
    pub max_state_age_ms: i64,
    /// 单跳深度下限（保留字段，供后续按腿估算容量）。
    pub min_hop_liquidity_usd: Decimal,
}

impl Default for PoolLimits {
    fn default() -> Self {
        Self {
            min_tvl_usd: Decimal::from(50_000),
            max_state_age_ms: 3_000,
            min_hop_liquidity_usd: Decimal::from(10_000),
        }
    }
}

/// Token 准入注册表：稳定排序输出，便于确定性搜索与复现。
#[derive(Clone, Debug, Default)]
pub struct TokenRegistry {
    admissions: HashMap<String, TokenAdmission>,
    ordered: Vec<String>,
}

impl TokenRegistry {
    /// 由 Token 注册表构建。显式状态只会被更严格的规则降级，不会被放宽。
    pub fn build(tokens: &HashMap<String, Token>) -> Self {
        let mut admissions = HashMap::new();
        let mut ordered: Vec<String> = tokens.keys().cloned().collect();
        ordered.sort();
        for symbol in &ordered {
            let token = &tokens[symbol];
            admissions.insert(symbol.clone(), assess_token(token));
        }
        Self {
            admissions,
            ordered,
        }
    }

    pub fn get(&self, symbol: &str) -> Option<&TokenAdmission> {
        self.admissions.get(symbol)
    }

    /// 未登记的 Token 一律视为 UNKNOWN。
    pub fn status(&self, symbol: &str) -> TokenStatus {
        self.admissions
            .get(symbol)
            .map(|a| a.status)
            .unwrap_or(TokenStatus::Unknown)
    }

    pub fn is_approved(&self, symbol: &str) -> bool {
        self.status(symbol).is_approved()
    }

    /// 可作为路线中间节点。
    pub fn hop_ok(&self, symbol: &str) -> bool {
        self.admissions
            .get(symbol)
            .map(TokenAdmission::hop_ok)
            .unwrap_or(false)
    }

    /// 可作为起始资产。
    pub fn start_ok(&self, symbol: &str) -> bool {
        self.admissions
            .get(symbol)
            .map(TokenAdmission::start_ok)
            .unwrap_or(false)
    }

    pub fn symbols(&self) -> &[String] {
        &self.ordered
    }

    /// (已批准, 全部)。
    pub fn counts(&self) -> (usize, usize) {
        let approved = self
            .admissions
            .values()
            .filter(|a| a.status.is_approved())
            .count();
        (approved, self.admissions.len())
    }
}

/// 由 Token 记录推导有效准入状态（只收紧，不放宽）。
pub fn assess_token(token: &Token) -> TokenAdmission {
    let mut reasons = Vec::new();
    let mut status = token.status;

    if !token.enabled {
        reasons.push("Token 已禁用".to_string());
        return TokenAdmission {
            symbol: token.symbol.clone(),
            status: TokenStatus::Rejected,
            allowed_start: false,
            allowed_hop: false,
            allowed_flash: false,
            reasons,
        };
    }

    if token.standard != "ERC20" {
        reasons.push(format!("非标准 ERC-20（{}）", token.standard));
        status = TokenStatus::Restricted;
    }
    if token.special_transfer {
        reasons.push("存在转账税/转账限制".to_string());
        status = TokenStatus::Restricted;
    }
    for tag in &token.risk_tags {
        if HARD_RISK_TAGS.iter().any(|t| t == tag) {
            reasons.push(format!("风险标签：{tag}"));
            status = TokenStatus::Restricted;
        }
    }
    match status {
        TokenStatus::Rejected => reasons.push("注册表标记为 REJECTED".to_string()),
        TokenStatus::Unknown => reasons.push("未审核（UNKNOWN）".to_string()),
        TokenStatus::Observing => reasons.push("观察中（OBSERVING）".to_string()),
        _ => {}
    }

    TokenAdmission {
        symbol: token.symbol.clone(),
        status,
        allowed_start: token.allowed_start,
        allowed_hop: token.allowed_hop,
        allowed_flash: token.allowed_flash,
        reasons,
    }
}

/// 单池准入判定。返回原因链，便于解释「为什么这个池没有出现在图里」。
pub fn assess_pool(
    rt: &PoolRuntime,
    registry: &TokenRegistry,
    limits: &PoolLimits,
    now: DateTime<Utc>,
) -> PoolAdmission {
    let mut reasons = Vec::new();
    if !rt.pool.verified {
        reasons.push("池未验证".to_string());
    }
    if !rt.pool.execution_allowed {
        reasons.push("池被禁止执行".to_string());
    }
    if !rt.pool.kind.execution_allowed() {
        reasons.push(format!("AMM 类型不可执行：{}", rt.pool.kind));
    }
    if !registry.hop_ok(&rt.pool.token0) {
        reasons.push(format!("Token {} 不满足准入", rt.pool.token0));
    }
    if !registry.hop_ok(&rt.pool.token1) {
        reasons.push(format!("Token {} 不满足准入", rt.pool.token1));
    }
    if rt.pool.tvl_usd < limits.min_tvl_usd {
        reasons.push(format!(
            "TVL {} USDC 低于门槛 {} USDC",
            rt.pool.tvl_usd, limits.min_tvl_usd
        ));
    }
    let age = rt.state.age_ms(now);
    if age > limits.max_state_age_ms {
        reasons.push(format!(
            "状态年龄 {age}ms 超过上限 {}ms",
            limits.max_state_age_ms
        ));
    }
    match rt.pool.kind {
        PoolKind::UniswapV2 | PoolKind::StableSwap => {
            if rt.state.reserve0 <= Decimal::ZERO || rt.state.reserve1 <= Decimal::ZERO {
                reasons.push("储备为零".to_string());
            }
        }
        PoolKind::UniswapV3 => {
            if rt.state.liquidity == 0 || rt.state.sqrt_price_x96 == 0 {
                reasons.push("V3 流动性或价格为空".to_string());
            }
        }
        PoolKind::WeightedPool => {
            if rt.state.reserve0 <= Decimal::ZERO
                || rt.state.reserve1 <= Decimal::ZERO
                || rt.state.weight0 <= Decimal::ZERO
                || rt.state.weight1 <= Decimal::ZERO
            {
                reasons.push("储备或权重非法".to_string());
            }
        }
        PoolKind::Unknown => {}
    }
    if rt.pool.token0 == rt.pool.token1 {
        reasons.push("退化池（token0 == token1）".to_string());
    }

    PoolAdmission {
        pool_id: rt.pool.pool_id.clone(),
        eligible: reasons.is_empty(),
        tvl_usd: rt.pool.tvl_usd,
        reasons,
    }
}

/// 池准入违规转为风控拒绝项（供已有机会解释原因）。
pub fn pool_violation(admission: &PoolAdmission) -> RiskViolation {
    RiskViolation {
        key: "pool_admission".into(),
        label: "池准入".into(),
        severity: RiskSeverity::HardStop,
        limit: Decimal::ONE,
        actual: Decimal::ZERO,
        unit: "bool".into(),
        message: format!(
            "池 {} 未通过准入：{}",
            admission.pool_id,
            admission.reasons.join("；")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;
    use crate::domain::market::{PoolRef, PoolState, Token};
    use crate::dex::math;

    fn token(symbol: &str, status: TokenStatus) -> Token {
        Token {
            symbol: symbol.into(),
            chain_id: 1,
            address: format!("0x{symbol}"),
            decimals: 6,
            standard: "ERC20".into(),
            code_hash: "0x1".into(),
            status,
            risk_tags: vec![],
            allowed_start: true,
            allowed_hop: true,
            allowed_flash: true,
            special_transfer: false,
            enabled: true,
        }
    }

    fn runtime(kind: PoolKind, tvl: &str, age_ms: i64, verified: bool) -> PoolRuntime {
        let now = Utc::now();
        PoolRuntime {
            pool: PoolRef {
                pool_id: "p1".into(),
                chain_id: 1,
                protocol_id: "proto".into(),
                kind,
                address: "0xp1".into(),
                token0: "USDC".into(),
                token1: "WETH".into(),
                decimals0: 0,
                decimals1: 0,
                fee_bps: 30,
                tick_spacing: None,
                tvl_usd: dec(tvl),
                verified,
                execution_allowed: true,
                added_at: now,
            },
            state: PoolState {
                pool_id: "p1".into(),
                block_number: 1,
                block_hash: "0xblock".into(),
                updated_at: now - chrono::Duration::milliseconds(age_ms),
                reserve0: dec("1000000"),
                reserve1: dec("1000"),
                sqrt_price_x96: math::Q96,
                liquidity: 1_000_000,
                tick: 0,
                ticks: vec![],
                amplification: dec("100"),
                weight0: dec("0.5"),
                weight1: dec("0.5"),
                fee_bps: 30,
            },
        }
    }

    #[test]
    fn token_status_only_tightens() {
        let mut approved = token("USDC", TokenStatus::Approved);
        assert_eq!(assess_token(&approved).status, TokenStatus::Approved);
        approved.special_transfer = true;
        assert_eq!(assess_token(&approved).status, TokenStatus::Restricted);
        approved.special_transfer = false;
        approved.status = TokenStatus::Unknown;
        assert_eq!(assess_token(&approved).status, TokenStatus::Unknown);
        approved.status = TokenStatus::Approved;
        approved.enabled = false;
        let a = assess_token(&approved);
        assert_eq!(a.status, TokenStatus::Rejected);
        assert!(!a.allowed_hop);
    }

    #[test]
    fn registry_builds_from_tokens() {
        let mut tokens = HashMap::new();
        tokens.insert("USDC".to_string(), token("USDC", TokenStatus::Approved));
        tokens.insert("WETH".to_string(), token("WETH", TokenStatus::Approved));
        let registry = TokenRegistry::build(&tokens);
        assert!(registry.hop_ok("USDC"));
        assert!(registry.start_ok("WETH"));
        assert_eq!(registry.counts(), (2, 2));
        assert_eq!(registry.symbols(), ["USDC", "WETH"]);
        assert_eq!(registry.status("DAI"), TokenStatus::Unknown);
        assert!(!registry.hop_ok("DAI"));
    }

    #[test]
    fn pool_admission_reasons() {
        let mut tokens = HashMap::new();
        tokens.insert("USDC".to_string(), token("USDC", TokenStatus::Approved));
        tokens.insert("WETH".to_string(), token("WETH", TokenStatus::Approved));
        let registry = TokenRegistry::build(&tokens);
        let limits = PoolLimits::default();
        let now = Utc::now();

        assert!(assess_pool(&runtime(PoolKind::UniswapV2, "100000", 100, true), &registry, &limits, now).eligible);

        let unverified = assess_pool(&runtime(PoolKind::UniswapV2, "100000", 100, false), &registry, &limits, now);
        assert!(!unverified.eligible);
        assert!(unverified.reasons.iter().any(|r| r.contains("未验证")));

        let poor = assess_pool(&runtime(PoolKind::UniswapV2, "1000", 100, true), &registry, &limits, now);
        assert!(!poor.eligible);
        assert!(poor.reasons.iter().any(|r| r.contains("TVL")));

        let stale = assess_pool(&runtime(PoolKind::UniswapV2, "100000", 10_000, true), &registry, &limits, now);
        assert!(!stale.eligible);

        let mut unknown_kind = runtime(PoolKind::Unknown, "100000", 100, true);
        unknown_kind.pool.kind = PoolKind::Unknown;
        assert!(!assess_pool(&unknown_kind, &registry, &limits, now).eligible);

        let v = pool_violation(&poor);
        assert_eq!(v.severity, RiskSeverity::HardStop);
        assert!(v.message.contains("p1"));
    }
}