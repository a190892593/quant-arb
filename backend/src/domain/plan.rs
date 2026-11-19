//! 交易计划：机会 → 可执行计划。Paper 与未来 Live 共用同一格式（TDD §1.1）。
//!
//! 计划必须带唯一 ID、有效期和幂等约束（PRD §7 安全验收）。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::market::RouteLeg;
use super::opportunity::FundingSource;
use super::types::{Direction, HedgeOrder, OrderType, Side, StrategyType};
// 计划状态。`Signature`/`Nonce` 在第一阶段仅本地模拟，不产生真实签名。
crate::str_enum! {
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PlanState {
    Draft => "DRAFT",
    RiskChecked => "RISK_CHECKED",
    Approved => "APPROVED",
    Executing => "EXECUTING",
    Completed => "COMPLETED",
    Rejected => "REJECTED",
    Expired => "EXPIRED",
}
}

/// CEX 腿执行计划。数量以实际成交为准，不以请求数量为准（TDD §7.3）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CexOrderPlan {
    pub exchange: String,
    pub symbol: String,
    pub base: String,
    pub quote: String,
    pub side: Side,
    pub qty: Decimal,
    /// 价格保护上限/下限（买为上限，卖为下限）。
    pub limit_price: Decimal,
    /// 参考价（下单时的对手价），用于计算最大偏离。
    pub reference_price: Decimal,
    pub max_slippage_bps: Decimal,
    pub order_type: OrderType,
    /// IOC 允许部分成交；FOK 必须全成。
    pub allow_partial: bool,
    pub client_order_id: String,
}

/// 链上腿计划摘要。第一阶段不广播，只做本地模拟。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DexLegPlan {
    pub route_id: String,
    pub chain_id: u64,
    pub amount_in: Decimal,
    pub min_out: Decimal,
    pub gas_limit: u64,
    pub max_gas_price_gwei: Decimal,
    pub max_block_number: u64,
    pub legs: Vec<RouteLeg>,
    /// 计划哈希：绑定路线、金额、最低输出与期限。
    pub route_hash: String,
}

/// 统一交易计划。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradePlan {
    pub plan_id: String,
    pub opportunity_id: String,
    pub strategy_type: StrategyType,
    pub chain_id: u64,
    pub direction: Option<Direction>,
    pub hedge_order: Option<HedgeOrder>,

    pub target_notional: Decimal,
    pub dex_leg: Option<DexLegPlan>,
    pub cex_leg: Option<CexOrderPlan>,
    pub funding: FundingSource,

    /// 起始资产计价的最低净利润（链上可验证部分）。
    pub min_profit_usd: Decimal,
    pub expected_net_profit_usd: Decimal,

    pub nonce: u64,
    pub deadline: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub state: PlanState,

    /// 计划哈希（幂等键）。同一计划重复提交必须命中同一哈希。
    pub plan_hash: String,
    /// 本地模拟签名标识；第一阶段不存在真实签名器。
    pub signer: String,
    pub signature: String,
}

impl TradePlan {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.deadline
    }

    /// 计划是否处于可执行状态。
    pub fn executable(&self, now: DateTime<Utc>) -> bool {
        matches!(self.state, PlanState::Approved | PlanState::RiskChecked) && !self.is_expired(now)
    }
}

/// 计划哈希输入：任何影响执行的字段变化都必须改变哈希。
pub fn plan_hash_input(plan: &TradePlan) -> String {
    let mut s = String::new();
    s.push_str(&plan.plan_id);
    s.push('|');
    s.push_str(&plan.opportunity_id);
    s.push('|');
    s.push_str(plan.strategy_type.as_str());
    s.push('|');
    s.push_str(&plan.chain_id.to_string());
    s.push('|');
    s.push_str(&plan.target_notional.to_string());
    s.push('|');
    s.push_str(&plan.nonce.to_string());
    s.push('|');
    s.push_str(&plan.deadline.timestamp_millis().to_string());
    s.push('|');
    if let Some(dex) = &plan.dex_leg {
        s.push_str(&dex.route_id);
        s.push(':');
        s.push_str(&dex.amount_in.to_string());
        s.push(':');
        s.push_str(&dex.min_out.to_string());
        s.push(':');
        s.push_str(&dex.route_hash);
        s.push(':');
        s.push_str(&dex.max_block_number.to_string());
    }
    s.push('|');
    if let Some(cex) = &plan.cex_leg {
        s.push_str(&cex.exchange);
        s.push('/');
        s.push_str(&cex.symbol);
        s.push(':');
        s.push_str(cex.side.as_str());
        s.push(':');
        s.push_str(&cex.qty.to_string());
        s.push(':');
        s.push_str(&cex.limit_price.to_string());
        s.push(':');
        s.push_str(&cex.client_order_id);
    }
    s.push('|');
    match &plan.funding {
        FundingSource::OwnCapital => s.push_str("OWN"),
        FundingSource::FlashLoan {
            provider,
            token,
            fee_amount,
            ..
        } => {
            s.push_str("FLASH:");
            s.push_str(provider);
            s.push(':');
            s.push_str(token);
            s.push(':');
            s.push_str(&fee_amount.to_string());
        }
    }
    s
}

/// 稳定的 64 位哈希（FNV-1a），用于路线/计划指纹。非密码学用途。
pub fn fnv1a_64(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 生成十六进制指纹字符串。
pub fn fingerprint(input: &str) -> String {
    format!("{:016x}", fnv1a_64(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    fn base_plan() -> TradePlan {
        TradePlan {
            plan_id: "plan_1".into(),
            opportunity_id: "opp_1".into(),
            strategy_type: StrategyType::DexTriple,
            chain_id: 42161,
            direction: None,
            hedge_order: None,
            target_notional: dec("1000"),
            dex_leg: None,
            cex_leg: None,
            funding: FundingSource::OwnCapital,
            min_profit_usd: dec("2"),
            expected_net_profit_usd: dec("3.4"),
            nonce: 7,
            deadline: Utc::now(),
            created_at: Utc::now(),
            state: PlanState::Approved,
            plan_hash: String::new(),
            signer: "paper".into(),
            signature: "local".into(),
        }
    }

    #[test]
    fn hash_changes_with_min_out() {
        let mut a = base_plan();
        let mut b = base_plan();
        a.dex_leg = Some(DexLegPlan {
            route_id: "r1".into(),
            chain_id: 42161,
            amount_in: dec("1000"),
            min_out: dec("1005"),
            gas_limit: 400_000,
            max_gas_price_gwei: dec("0.1"),
            max_block_number: 100,
            legs: vec![],
            route_hash: "abc".into(),
        });
        b.dex_leg = a.dex_leg.clone();
        if let Some(d) = b.dex_leg.as_mut() {
            d.min_out = dec("1004");
        }
        assert_ne!(
            fingerprint(&plan_hash_input(&a)),
            fingerprint(&plan_hash_input(&b))
        );
    }

    #[test]
    fn hash_is_stable() {
        let a = base_plan();
        assert_eq!(
            fingerprint(&plan_hash_input(&a)),
            fingerprint(&plan_hash_input(&a))
        );
    }
}
