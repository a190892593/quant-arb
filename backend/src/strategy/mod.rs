//! 统一策略引擎（TDD §6）。
//!
//! 一个机会对象、一套成本模型、一套评分、一套风控判定，
//! 供 Research / Paper / 未来 Live 共用。此模块只做「发现与定价」，
//! 执行与结算在 [`crate::paper`]。

pub mod cex_dex;
pub mod cost;
pub mod dex_arb;
pub mod dex_scan;
pub mod scorer;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::cex::BookEngine;
use crate::domain::market::{Instrument, RiskDecision, RiskViolation, Token};
use crate::domain::opportunity::{FundingSource, Opportunity};
use crate::domain::risk::{ControlState, RiskInput};
use crate::domain::types::{
    DataQuality, OppStatus, OpportunityLifecycle, RiskSeverity, StrategyType,
};
use crate::dex::PoolSet;
use crate::risk::RiskEngine;

/// 扫描时看到的市场视图。所有策略都只读这一结构。
pub struct MarketView<'a> {
    pub now: DateTime<Utc>,
    pub chain_id: u64,
    pub chain_name: String,
    pub head_block: u64,
    pub pools: &'a PoolSet,
    pub tokens: &'a HashMap<String, Token>,
    pub prices: &'a HashMap<String, Decimal>,
    pub books: &'a BookEngine,
    pub instruments: &'a HashMap<String, Instrument>,
}

impl<'a> MarketView<'a> {
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
    pub fn chain_name(&self) -> &str {
        &self.chain_name
    }
}

/// DEX 原子策略参数。
#[derive(Clone, Debug)]
pub struct DexCostParams {
    pub gas_price_gwei: Decimal,
    pub native_price_usd: Decimal,
    pub gas_per_hop: u64,
    pub flash_fee_bps: Decimal,
    pub flash_provider: String,
    pub priority_fee_usd: Decimal,
    pub delay_reserve_bps: Decimal,
    pub mev_reserve_bps: Decimal,
    pub adverse_bps: Decimal,
    pub profit_discount_pct: Decimal,
    pub ttl_sec: i64,
    pub max_hops: usize,
}

impl Default for DexCostParams {
    fn default() -> Self {
        Self {
            gas_price_gwei: Decimal::new(2, 2),
            native_price_usd: Decimal::from(2800),
            gas_per_hop: 130_000,
            flash_fee_bps: Decimal::new(5, 0),
            flash_provider: "aave-v3".into(),
            priority_fee_usd: Decimal::new(5, 2),
            delay_reserve_bps: Decimal::from(8),
            mev_reserve_bps: Decimal::from(8),
            adverse_bps: Decimal::from(25),
            profit_discount_pct: Decimal::from(50),
            ttl_sec: 30,
            max_hops: 4,
        }
    }
}

/// CEX–DEX 策略参数。
#[derive(Clone, Debug)]
pub struct CexDexParams {
    pub gas_price_gwei: Decimal,
    pub native_price_usd: Decimal,
    pub delay_reserve_bps: Decimal,
    pub compensation_bps: Decimal,
    pub rebalance_bps: Decimal,
    pub slippage_bps: Decimal,
    pub adverse_bps: Decimal,
    pub price_protection_bps: Decimal,
    pub depth_usage_max_pct: Decimal,
    pub profit_discount_pct: Decimal,
    pub ttl_sec: i64,
    pub max_hops: usize,
}

impl Default for CexDexParams {
    fn default() -> Self {
        Self {
            gas_price_gwei: Decimal::new(2, 2),
            native_price_usd: Decimal::from(2800),
            delay_reserve_bps: Decimal::from(6),
            compensation_bps: Decimal::from(6),
            rebalance_bps: Decimal::from(3),
            slippage_bps: Decimal::from(3),
            adverse_bps: Decimal::from(30),
            price_protection_bps: Decimal::from(10),
            depth_usage_max_pct: Decimal::from(10),
            profit_discount_pct: Decimal::from(50),
            ttl_sec: 30,
            max_hops: 2,
        }
    }
}

/// 扫描配置。
#[derive(Clone, Debug)]
pub struct ScanConfig {
    pub notional_ladder: Vec<Decimal>,
    pub base_symbols: Vec<String>,
    /// 每轮在 DEX 与 CEX–DEX 之间是否翻转执行顺序假设。
    pub cex_first: bool,
}

impl Default for ScanConfig {
    fn default() -> Self {
        let ladder = [250u64, 500, 1_000, 2_500]
            .iter()
            .map(|v| Decimal::from(*v))
            .collect();
        Self {
            notional_ladder: ladder,
            base_symbols: vec!["USDC".into(), "USDT".into()],
            cex_first: true,
        }
    }
}

/// 每次评估需要的账户与风险上下文。
#[derive(Clone, Debug)]
pub struct ScanContext {
    pub equity_usd: Decimal,
    pub peak_equity_usd: Decimal,
    pub account_daily_loss_usd: Decimal,
    pub strategy_daily_loss: HashMap<StrategyType, Decimal>,
    pub unrealized_pnl_usd: Decimal,
    pub controls: ControlState,
    pub head_block: u64,
    pub dex_available: HashMap<String, Decimal>,
    pub cex_available: HashMap<String, Decimal>,
    /// 历史成交率（用于评分），按策略给出。
    pub fill_rate: HashMap<StrategyType, Decimal>,
}

impl ScanContext {
    pub fn strategy_loss(&self, s: StrategyType) -> Decimal {
        self.strategy_daily_loss.get(&s).copied().unwrap_or(Decimal::ZERO)
    }
    pub fn fill_rate_of(&self, s: StrategyType) -> Decimal {
        self.fill_rate
            .get(&s)
            .copied()
            .unwrap_or(Decimal::new(95, 2))
    }
}

/// 策略引擎本体。风控引擎按调用传入，避免持有可变全局状态。
pub struct StrategyEngine {
    pub dex: DexCostParams,
    pub cex_dex: CexDexParams,
}

impl StrategyEngine {
    pub fn new() -> Self {
        Self {
            dex: DexCostParams::default(),
            cex_dex: CexDexParams::default(),
        }
    }

    /// 扫描全部策略，返回已完成风控判定的机会（含被拒绝的机会）。
    pub fn scan(
        &self,
        view: &MarketView<'_>,
        cfg: &ScanConfig,
        ctx: &ScanContext,
        risk: &RiskEngine,
    ) -> Vec<Opportunity> {
        let mut out = Vec::new();
        let bases: Vec<&str> = cfg.base_symbols.iter().map(|s| s.as_str()).collect();

        for notional in &cfg.notional_ladder {
            // DEX 原子：可用于判断自有资金是否足够
            let mut dex_opps = dex_arb::scan(
                view,
                &self.dex,
                &bases,
                *notional,
                &ctx.dex_available,
            );
            for o in dex_opps.drain(..) {
                out.push(self.apply_risk_and_score(view, ctx, risk, o));
            }

            // CEX–DEX：库存为两边预置
            let mut cex_opps = cex_dex::scan(
                view,
                &self.cex_dex,
                *notional,
                &ctx.cex_available,
                cfg.cex_first,
            );
            for o in cex_opps.drain(..) {
                out.push(self.apply_risk_and_score(view, ctx, risk, o));
            }
        }

        // 同一路线 + 同一规模只保留一条，按净利降序
        out.sort_by(|a, b| {
            b.conservative_net_profit
                .cmp(&a.conservative_net_profit)
        });
        out
    }

    /// 只扫描 CEX–DEX 策略（DEX 原子扫描由 [dex_scan] 新引擎负责）。
    pub fn scan_cex_dex(
        &self,
        view: &MarketView<'_>,
        cfg: &ScanConfig,
        ctx: &ScanContext,
        risk: &RiskEngine,
    ) -> Vec<Opportunity> {
        let mut out = Vec::new();
        for notional in &cfg.notional_ladder {
            let mut opps = cex_dex::scan(
                view,
                &self.cex_dex,
                *notional,
                &ctx.cex_available,
                cfg.cex_first,
            );
            for o in opps.drain(..) {
                out.push(self.apply_risk_and_score(view, ctx, risk, o));
            }
        }
        out.sort_by(|a, b| b.conservative_net_profit.cmp(&a.conservative_net_profit));
        out
    }

    /// 风控判定 + 评分；返回值状态反映真实可执行性。
    pub fn apply_risk_and_score(
        &self,
        view: &MarketView<'_>,
        ctx: &ScanContext,
        risk: &RiskEngine,
        mut opp: Opportunity,
    ) -> Opportunity {
        let min_profit = risk.min_profit_for(opp.strategy_type, opp.target_notional);

        let (book_valid, book_age, book_depth, depth_usage) = match (
            opp.cex_exchange.as_deref(),
            opp.cex_symbol.as_deref(),
        ) {
            (Some(x), Some(s)) => {
                let book = view.books.get(x, s);
                let valid = book.map(|b| b.valid);
                let age = book.map(|b| (view.now - b.recv_ts).num_milliseconds());
                let depth = book.map(|b| b.depth_usd(crate::domain::types::Side::Buy));
                let usage = book.and_then(|b| {
                    if opp.target_notional.is_zero() {
                        None
                    } else {
                        Some(opp.target_notional / b.depth_usd(crate::domain::types::Side::Buy) * Decimal::from(100))
                    }
                });
                (valid, age, depth, usage)
            }
            _ => (None, None, None, None),
        };

        let input = RiskInput {
            strategy: opp.strategy_type,
            chain_id: opp.chain_id,
            exchange: opp.cex_exchange.clone(),
            symbol: opp.cex_symbol.clone(),
            tokens: opp
                .dex_route
                .as_ref()
                .map(|r| r.legs.iter().map(|l| l.token_in.clone()).collect())
                .unwrap_or_default(),
            target_notional: opp.target_notional,
            min_profit_usd: min_profit,
            net_profit_usd: opp.conservative_net_profit,
            gross_profit_usd: opp.executable_gross_profit,
            gas_usd: opp.costs.get("gas"),
            price_impact_bps: opp
                .dex_route
                .as_ref()
                .map(|r| r.price_impact_bps)
                .unwrap_or(Decimal::ZERO),
            quote_age_ms: opp.quote_age_ms,
            quote_block_number: opp.quote_block_number,
            head_block_number: ctx.head_block,
            book_age_ms: book_age,
            book_valid,
            book_depth_usd: book_depth,
            depth_usage_pct: depth_usage,
            fee_known: opp
                .cex_exchange
                .as_ref()
                .and_then(|_| opp.cex_symbol.as_ref())
                .and_then(|s| view.instruments.get(s))
                .map(|i| i.fee_known),
            data_quality: opp.data_quality,
            equity_usd: ctx.equity_usd,
            strategy_daily_loss_usd: ctx.strategy_loss(opp.strategy_type),
            account_daily_loss_usd: ctx.account_daily_loss_usd,
            peak_equity_usd: ctx.peak_equity_usd,
            unrealized_pnl_usd: ctx.unrealized_pnl_usd,
            required_inventory: opp
                .required_dex_inventory
                .iter()
                .chain(opp.required_cex_inventory.iter())
                .cloned()
                .collect(),
            funding: Some(opp.funding.clone()),
            flash_fee_known: !matches!(opp.funding, FundingSource::FlashLoan { .. })
                || !self.dex.flash_fee_bps.is_zero(),
        };

        // 路线级门禁（Token/Pool 准入、状态一致性、M1 模拟、容量）在引擎中已产生，
        // 这里与风控引擎的判定合并；WARNING 不阻断，只记录。
        let pre: Vec<RiskViolation> = std::mem::take(&mut opp.rejections);
        let mut decision: RiskDecision = risk.evaluate(&input, &ctx.controls, view.now);
        for v in pre {
            if v.severity == RiskSeverity::Warning {
                decision.warnings.push(v);
            } else {
                decision.allowed = false;
                decision.violations.push(v);
            }
        }

        // 评分
        let score = scorer::score(&scorer::ScoreInput {
            net_profit_usd: opp.conservative_net_profit,
            min_profit_usd: min_profit,
            model_level: opp.model_level,
            capacity_usd: opp.capacity_usd,
            target_notional: opp.target_notional,
            price_impact_bps: input.price_impact_bps,
            data_quality: opp.data_quality,
            quote_age_ms: opp.quote_age_ms,
            book_age_ms: book_age,
            risk_tags: 0,
            historical_fill_rate: ctx.fill_rate_of(opp.strategy_type),
            depth_usage_pct: depth_usage,
        });
        opp.risk_score = score;
        opp.rejections = decision.violations.clone();

        if !decision.allowed {
            opp.status = OppStatus::Rejected;
        } else if opp.conservative_net_profit <= Decimal::ZERO {
            // 净利润不为正的机会只能是观察项
            opp.status = OppStatus::Watch;
        } else if opp.capacity_usd < opp.target_notional {
            opp.status = OppStatus::Rejected;
        } else if opp.data_quality.rank() >= DataQuality::Low.rank() {
            opp.status = OppStatus::Rejected;
        } else {
            opp.status = OppStatus::Actionable;
        }
        if opp.status == OppStatus::Rejected {
            opp.lifecycle = OpportunityLifecycle::Rejected;
        } else if opp.lifecycle == OpportunityLifecycle::Rejected {
            // 门禁曾拒绝但风控放开：保持拒绝，避免状态倒退造成误执行。
            opp.status = OppStatus::Rejected;
        }
        opp
    }
}

/// 判断机会是否仍可作为可执行计划（供 Paper 引擎调用）。
pub fn is_executable(opp: &Opportunity, now: DateTime<Utc>, min_profit: Decimal) -> bool {
    opp.status == OppStatus::Actionable
        && opp.is_fresh(now)
        && opp.conservative_net_profit >= min_profit
        && opp.conservative_net_profit > Decimal::ZERO
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_defaults_are_conservative() {
        let d = DexCostParams::default();
        assert_eq!(d.max_hops, 4);
        assert_eq!(d.profit_discount_pct, Decimal::from(50));
        let c = CexDexParams::default();
        assert_eq!(c.price_protection_bps, Decimal::from(10));
        assert_eq!(c.depth_usage_max_pct, Decimal::from(10));
    }

    #[test]
    fn executable_requires_positive_net_and_freshness() {
        use crate::domain::new_id;
        let now = Utc::now();
        let opp = Opportunity {
            opportunity_id: new_id("opp"),
            strategy_type: StrategyType::DexTriple,
            chain_id: 1,
            chain_name: "x".into(),
            dex_route: None,
            cex_exchange: None,
            cex_symbol: None,
            direction: None,
            target_notional: Decimal::from(1000),
            target_qty: Decimal::new(3579, 4),
            required_dex_inventory: vec![],
            required_cex_inventory: vec![],
            theoretical_edge_bps: Decimal::ZERO,
            executable_gross_profit: Decimal::from(10),
            total_cost: Decimal::from(8),
            conservative_net_profit: Decimal::from(2),
            stress_net_profit: Decimal::ZERO,
            worst_case_loss: Decimal::ZERO,
            costs: cost::CostParts::new().breakdown(),
            scenario: crate::domain::market::ProfitScenario::zero(),
            risk_score: crate::domain::market::RiskScore::from_components(vec![]),
            data_quality: DataQuality::High,
            model_level: crate::domain::market::ModelLevel::M2,
            confidence: String::new(),
            quote_timestamp: now,
            quote_block_number: 1,
            book_age_ms: 0,
            quote_age_ms: 0,
            status: OppStatus::Actionable,
            lifecycle: crate::domain::types::OpportunityLifecycle::Simulated,
            route_fingerprint: None,
            rejections: vec![],
            capacity_usd: Decimal::from(5000),
            funding: FundingSource::OwnCapital,
            hedge_order: None,
            detected_at: now,
            expires_at: now + chrono::Duration::seconds(30),
        };
        assert!(is_executable(&opp, now, Decimal::from(2)));
        assert!(!is_executable(&opp, now, Decimal::from(3)));
        assert!(!is_executable(&opp, now + chrono::Duration::seconds(31), Decimal::from(1)));
    }
}
