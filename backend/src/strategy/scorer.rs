//! 机会评分（TDD §6.3）。
//!
//! 权重：保守净利润 25、模拟可信度 20、流动性容量 15、执行竞争 15、
//! 数据新鲜度 10、Token/Pool/Exchange 风险 10、历史执行表现 5。
//!
//! 评分**不能替代**硬性风控拦截：即使 100 分，触发 HARD_STOP 仍然拒绝。

use rust_decimal::Decimal;

use crate::domain::market::{DataQuality, ModelLevel, RiskScore, ScoreComponent};

/// 评分输入。
#[derive(Clone, Debug)]
pub struct ScoreInput {
    /// 保守净利润（USDC）。
    pub net_profit_usd: Decimal,
    /// 该策略的最低净利润门槛。
    pub min_profit_usd: Decimal,
    pub model_level: ModelLevel,
    /// 可成交容量（USDC）。
    pub capacity_usd: Decimal,
    pub target_notional: Decimal,
    /// 价格冲击（bps）。
    pub price_impact_bps: Decimal,
    pub data_quality: DataQuality,
    /// 报价年龄 / 盘口年龄（毫秒）。
    pub quote_age_ms: i64,
    pub book_age_ms: Option<i64>,
    /// Token 风险标签数量（未验证、特殊转账、黑名单风险等）。
    pub risk_tags: usize,
    /// 历史执行表现：成交率（0–1）。
    pub historical_fill_rate: Decimal,
    /// 盘口深度占用比例（%），仅 CEX–DEX。
    pub depth_usage_pct: Option<Decimal>,
}

fn w_net() -> Decimal {
    Decimal::from(25)
}
fn w_confidence() -> Decimal {
    Decimal::from(20)
}
fn w_capacity() -> Decimal {
    Decimal::from(15)
}
fn w_competition() -> Decimal {
    Decimal::from(15)
}
fn w_freshness() -> Decimal {
    Decimal::from(10)
}
fn w_risk() -> Decimal {
    Decimal::from(10)
}
fn w_history() -> Decimal {
    Decimal::from(5)
}

fn clamp01(v: Decimal) -> Decimal {
    v.max(Decimal::ZERO).min(Decimal::ONE)
}

/// 计算评分（0–100）。
pub fn score(input: &ScoreInput) -> RiskScore {
    let mut parts = Vec::with_capacity(7);

    // 1) 保守净利润：达到门槛得 60%，达到 3 倍门槛得满分。
    let ratio = if input.min_profit_usd.is_zero() {
        Decimal::ZERO
    } else {
        input.net_profit_usd / input.min_profit_usd
    };
    let net_points = if ratio <= Decimal::ZERO {
        Decimal::ZERO
    } else if ratio <= Decimal::ONE {
        w_net() * Decimal::new(6, 1) * clamp01(ratio)
    } else {
        w_net()
            * (Decimal::new(6, 1)
                + Decimal::new(4, 1) * clamp01((ratio - Decimal::ONE) / Decimal::from(2)))
    };
    parts.push(ScoreComponent {
        key: "net_profit".into(),
        label: "保守净利润".into(),
        points: net_points.round_dp(2),
        max: w_net(),
        note: Some(format!("净利润/门槛 = {:.2}", ratio.round_dp(2))),
    });

    // 2) 模拟可信度：M0 25%、M1 50%、M2 85%、M3 100%
    let conf_ratio = match input.model_level {
        ModelLevel::M0 => Decimal::new(25, 2),
        ModelLevel::M1 => Decimal::new(50, 2),
        ModelLevel::M2 => Decimal::new(85, 2),
        ModelLevel::M3 => Decimal::ONE,
    };
    parts.push(ScoreComponent {
        key: "confidence".into(),
        label: "模拟可信度".into(),
        points: (w_confidence() * conf_ratio).round_dp(2),
        max: w_confidence(),
        note: Some(format!("模型等级 {}", input.model_level)),
    });

    // 3) 流动性容量：容量覆盖目标名义的比例。
    let cap_ratio = if input.target_notional.is_zero() {
        Decimal::ZERO
    } else {
        clamp01(input.capacity_usd / input.target_notional)
    };
    parts.push(ScoreComponent {
        key: "capacity".into(),
        label: "流动性容量".into(),
        points: (w_capacity() * cap_ratio).round_dp(2),
        max: w_capacity(),
        note: Some(format!("容量 {} USDC", input.capacity_usd.round_dp(0))),
    });

    // 4) 执行竞争：冲击与深度占用越高，竞争/抢跑风险越大。
    let impact_penalty = clamp01(input.price_impact_bps / Decimal::from(50));
    let depth_penalty = input
        .depth_usage_pct
        .map(|d| clamp01(d / Decimal::from(30)))
        .unwrap_or(Decimal::ZERO);
    let competition_ratio =
        clamp01(Decimal::ONE - (impact_penalty + depth_penalty) / Decimal::from(2));
    parts.push(ScoreComponent {
        key: "competition".into(),
        label: "执行竞争".into(),
        points: (w_competition() * competition_ratio).round_dp(2),
        max: w_competition(),
        note: Some(format!("冲击 {:.1} bps", input.price_impact_bps.round_dp(1))),
    });

    // 5) 数据新鲜度：250ms 内满分，超过 1s 归零。
    let age = input
        .book_age_ms
        .map(|b| b.max(input.quote_age_ms))
        .unwrap_or(input.quote_age_ms)
        .max(0);
    let fresh_ratio = if age <= 250 {
        Decimal::ONE
    } else if age >= 1_000 {
        Decimal::ZERO
    } else {
        Decimal::ONE - Decimal::from(age - 250) / Decimal::from(750)
    };
    parts.push(ScoreComponent {
        key: "freshness".into(),
        label: "数据新鲜度".into(),
        points: (w_freshness() * fresh_ratio).round_dp(2),
        max: w_freshness(),
        note: Some(format!("数据年龄 {age} ms")),
    });

    // 6) Token/Pool/交易所风险：风险标签每多一个扣 25%，数据质量非 A/B 再扣。
    let tag_penalty = clamp01(Decimal::from(input.risk_tags as u64) * Decimal::new(25, 2));
    let quality_penalty = match input.data_quality {
        DataQuality::Verified => Decimal::ZERO,
        DataQuality::High => Decimal::new(1, 1),
        DataQuality::Medium => Decimal::new(3, 1),
        DataQuality::Low => Decimal::new(6, 1),
        DataQuality::Invalid => Decimal::ONE,
    };
    let risk_ratio = clamp01(Decimal::ONE - tag_penalty - quality_penalty);
    parts.push(ScoreComponent {
        key: "asset_risk".into(),
        label: "资产与协议风险".into(),
        points: (w_risk() * risk_ratio).round_dp(2),
        max: w_risk(),
        note: Some(format!(
            "风险标签 {} 个，数据质量 {}",
            input.risk_tags, input.data_quality
        )),
    });

    // 7) 历史执行表现：成交率直接映射。
    let history_ratio = clamp01(input.historical_fill_rate);
    parts.push(ScoreComponent {
        key: "history".into(),
        label: "历史执行表现".into(),
        points: (w_history() * history_ratio).round_dp(2),
        max: w_history(),
        note: Some(format!(
            "历史成交率 {:.1}%",
            history_ratio * Decimal::from(100)
        )),
    });

    RiskScore::from_components(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    fn baseline() -> ScoreInput {
        ScoreInput {
            net_profit_usd: dec("6.0"),
            min_profit_usd: dec("2.0"),
            model_level: ModelLevel::M3,
            capacity_usd: dec("5000"),
            target_notional: dec("1000"),
            price_impact_bps: dec("5"),
            data_quality: DataQuality::Verified,
            quote_age_ms: 100,
            book_age_ms: Some(80),
            risk_tags: 0,
            historical_fill_rate: Decimal::ONE,
            depth_usage_pct: None,
        }
    }

    #[test]
    fn strong_opportunity_scores_high() {
        let s = score(&baseline());
        assert!(s.total >= dec("90"), "got {}", s.total);
        assert_eq!(s.grade, "A");
        assert_eq!(s.components.len(), 7);
    }

    #[test]
    fn weak_opportunity_scores_low() {
        let s = score(&ScoreInput {
            net_profit_usd: dec("0.1"),
            min_profit_usd: dec("5"),
            model_level: ModelLevel::M0,
            capacity_usd: dec("100"),
            target_notional: dec("1000"),
            price_impact_bps: dec("40"),
            data_quality: DataQuality::Low,
            quote_age_ms: 2_000,
            book_age_ms: Some(2_000),
            risk_tags: 3,
            historical_fill_rate: Decimal::ZERO,
            depth_usage_pct: Some(dec("28")),
        });
        assert!(s.total < dec("35"), "got {}", s.total);
        assert_eq!(s.grade, "E");
    }

    #[test]
    fn component_sums_to_total() {
        let s = score(&baseline());
        let sum: Decimal = s.components.iter().map(|c| c.points).sum();
        assert_eq!(sum.round_dp(2), s.total.round_dp(2));
    }

    #[test]
    fn each_component_within_max() {
        let s = score(&baseline());
        for c in &s.components {
            assert!(c.points <= c.max, "{} 超出上限", c.key);
            assert!(c.points >= Decimal::ZERO);
        }
    }
}
