//! 风控领域类型：参数、判定、事件与熔断控制状态。
//!
//! 参数目录与判定逻辑在 `crate::risk`；此处只定义跨模块共享的数据结构，
//! 避免数据库、API 与策略引擎各自解释阈值。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::market::RiskViolation;
use super::types::{ParamDir, RiskEventKind, RiskScope, RiskSeverity};

/// 一条风控参数基线（风控 §13 的字段子集）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiskParam {
    pub key: String,
    pub label: String,
    pub group: String,
    pub unit: String,
    pub severity: RiskSeverity,
    pub scope: RiskScope,
    pub dir: ParamDir,
    pub value: Decimal,
    /// 允许的取值区间，防止策略把参数设成无效值。
    pub min: Option<Decimal>,
    pub max: Option<Decimal>,
    /// 文档出处，例如 `§3`、`§4.2`。
    pub source: String,
    pub version: String,
    pub note: String,
}

/// 参数覆盖：只能比全局更严格（风控 §13）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParamOverride {
    pub key: String,
    pub scope: RiskScope,
    /// 作用域标识：策略 ID / 链 ID / 交易所 / 交易对 / Token。
    pub scope_ref: String,
    pub value: Decimal,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
    pub reason: String,
    pub version: u64,
}

/// 参数解析上下文。
#[derive(Clone, Debug, Default)]
pub struct ParamContext {
    pub strategy_id: Option<String>,
    pub chain_id: Option<u64>,
    pub exchange: Option<String>,
    pub symbol: Option<String>,
    pub token: Option<String>,
}

/// 参数版本信息（写入每次风控判定）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParamVersion {
    pub version: String,
    pub effective_from: DateTime<Utc>,
    pub updated_by: String,
    pub reason: String,
}

/// 风控事件（每次熔断、拒绝、恢复都必须留痕）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiskEvent {
    pub event_id: String,
    pub kind: RiskEventKind,
    pub severity: RiskSeverity,
    pub scope: RiskScope,
    pub scope_ref: String,
    pub message: String,
    pub detail: BTreeMap<String, String>,
    pub triggered_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
    pub resolved_by: Option<String>,
    pub opportunity_id: Option<String>,
    pub execution_id: Option<String>,
}

impl RiskEvent {
    pub fn is_active(&self) -> bool {
        self.resolved_at.is_none()
    }
}

/// 暂停原因。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PauseEntry {
    pub paused: bool,
    pub reason: String,
    pub at: DateTime<Utc>,
    pub by: String,
    pub until: Option<DateTime<Utc>>,
}

/// 五级熔断控制状态（PRD §5.8）：全局 / 策略 / 链 / CEX / 交易对与 Token。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ControlState {
    pub global: PauseEntry,
    pub strategies: BTreeMap<String, PauseEntry>,
    pub chains: BTreeMap<String, PauseEntry>,
    pub exchanges: BTreeMap<String, PauseEntry>,
    pub symbols: BTreeMap<String, PauseEntry>,
    pub tokens: BTreeMap<String, PauseEntry>,
    /// 最近一次状态变更时间。
    pub updated_at: Option<DateTime<Utc>>,
}

fn idle_entry() -> PauseEntry {
    PauseEntry {
        paused: false,
        reason: String::new(),
        at: Utc::now(),
        by: String::new(),
        until: None,
    }
}

impl ControlState {
    pub fn new() -> Self {
        Self {
            global: idle_entry(),
            ..Default::default()
        }
    }

    fn map_mut(&mut self, scope: RiskScope) -> Option<&mut BTreeMap<String, PauseEntry>> {
        match scope {
            RiskScope::Strategy => Some(&mut self.strategies),
            RiskScope::Chain => Some(&mut self.chains),
            RiskScope::Exchange => Some(&mut self.exchanges),
            RiskScope::Symbol => Some(&mut self.symbols),
            RiskScope::Token => Some(&mut self.tokens),
            RiskScope::Global => None,
        }
    }

    /// 设置/解除暂停。
    pub fn set(
        &mut self,
        scope: RiskScope,
        scope_ref: &str,
        paused: bool,
        reason: &str,
        by: &str,
        until: Option<DateTime<Utc>>,
    ) {
        let entry = PauseEntry {
            paused,
            reason: reason.to_string(),
            at: Utc::now(),
            by: by.to_string(),
            until,
        };
        match scope {
            RiskScope::Global => self.global = entry,
            _ => {
                if let Some(map) = self.map_mut(scope) {
                    if paused || map.contains_key(scope_ref) {
                        map.insert(scope_ref.to_string(), entry);
                    }
                }
            }
        }
        self.updated_at = Some(Utc::now());
    }

    /// 当前是否应拒绝新执行。
    ///
    /// 任一上级作用域暂停都会向下覆盖：全局 → 策略 → 链 → 交易所 → 交易对 → Token。
    pub fn is_blocked(
        &self,
        now: DateTime<Utc>,
        strategy: Option<&str>,
        chain: Option<u64>,
        exchange: Option<&str>,
        symbol: Option<&str>,
        token: Option<&str>,
    ) -> Option<String> {
        let active = |e: &PauseEntry| -> bool {
            e.paused && e.until.map(|u| now < u).unwrap_or(true) || (e.paused && e.until.is_none())
        };
        if active(&self.global) {
            return Some(format!("全局暂停：{}", self.global.reason));
        }
        if let Some(s) = strategy {
            if let Some(e) = self.strategies.get(s) {
                if active(e) {
                    return Some(format!("策略 {} 暂停：{}", s, e.reason));
                }
            }
        }
        if let Some(c) = chain {
            if let Some(e) = self.chains.get(&c.to_string()) {
                if active(e) {
                    return Some(format!("链 {} 暂停：{}", c, e.reason));
                }
            }
        }
        if let Some(x) = exchange {
            if let Some(e) = self.exchanges.get(x) {
                if active(e) {
                    return Some(format!("交易所 {} 暂停：{}", x, e.reason));
                }
            }
        }
        if let Some(s) = symbol {
            if let Some(e) = self.symbols.get(s) {
                if active(e) {
                    return Some(format!("交易对 {} 暂停：{}", s, e.reason));
                }
            }
        }
        if let Some(t) = token {
            if let Some(e) = self.tokens.get(t) {
                if active(e) {
                    return Some(format!("Token {} 暂停：{}", t, e.reason));
                }
            }
        }
        None
    }

    /// 所有处于暂停状态的作用域（用于前端展示）。
    pub fn active_pauses(&self, now: DateTime<Utc>) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        let active = |e: &PauseEntry| -> bool {
            e.paused && (e.until.is_none() || e.until.map(|u| now < u).unwrap_or(false))
        };
        if active(&self.global) {
            out.push(("GLOBAL".to_string(), "-".to_string(), self.global.reason.clone()));
        }
        for (scope, map) in [
            (RiskScope::Strategy, &self.strategies),
            (RiskScope::Chain, &self.chains),
            (RiskScope::Exchange, &self.exchanges),
            (RiskScope::Symbol, &self.symbols),
            (RiskScope::Token, &self.tokens),
        ] {
            for (k, e) in map.iter() {
                if active(e) {
                    out.push((scope.as_str().to_string(), k.clone(), e.reason.clone()));
                }
            }
        }
        out
    }
}

/// 一次风控评估的输入快照。由调用方（策略引擎 / Paper 引擎）填充。
#[derive(Clone, Debug)]
pub struct RiskInput {
    pub strategy: crate::domain::types::StrategyType,
    pub chain_id: u64,
    pub exchange: Option<String>,
    pub symbol: Option<String>,
    pub tokens: Vec<String>,
    pub target_notional: Decimal,
    pub min_profit_usd: Decimal,
    pub net_profit_usd: Decimal,
    pub gross_profit_usd: Decimal,
    pub gas_usd: Decimal,
    pub price_impact_bps: Decimal,
    pub quote_age_ms: i64,
    pub quote_block_number: u64,
    pub head_block_number: u64,
    pub book_age_ms: Option<i64>,
    pub book_valid: Option<bool>,
    pub book_depth_usd: Option<Decimal>,
    pub depth_usage_pct: Option<Decimal>,
    pub fee_known: Option<bool>,
    pub data_quality: super::types::DataQuality,
    /// 当前权益（USDC），用于比例型风控。
    pub equity_usd: Decimal,
    pub strategy_daily_loss_usd: Decimal,
    pub account_daily_loss_usd: Decimal,
    pub peak_equity_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub required_inventory: Vec<super::opportunity::InventoryNeed>,
    pub funding: Option<super::opportunity::FundingSource>,
    pub flash_fee_known: bool,
}

impl RiskInput {
    /// 仅为满足 `Default` 语义的最小构造，测试与占位使用。
    pub fn zero(strategy: crate::domain::types::StrategyType) -> Self {
        Self {
            strategy,
            chain_id: 0,
            exchange: None,
            symbol: None,
            tokens: vec![],
            target_notional: Decimal::ZERO,
            min_profit_usd: Decimal::ZERO,
            net_profit_usd: Decimal::ZERO,
            gross_profit_usd: Decimal::ZERO,
            gas_usd: Decimal::ZERO,
            price_impact_bps: Decimal::ZERO,
            quote_age_ms: 0,
            quote_block_number: 0,
            head_block_number: 0,
            book_age_ms: None,
            book_valid: None,
            book_depth_usd: None,
            depth_usage_pct: None,
            fee_known: None,
            data_quality: super::types::DataQuality::Invalid,
            equity_usd: Decimal::ZERO,
            strategy_daily_loss_usd: Decimal::ZERO,
            account_daily_loss_usd: Decimal::ZERO,
            peak_equity_usd: Decimal::ZERO,
            unrealized_pnl_usd: Decimal::ZERO,
            required_inventory: vec![],
            funding: None,
            flash_fee_known: false,
        }
    }
}

/// 便捷构造：生成一条拒绝原因。
pub fn violation(
    key: &str,
    label: &str,
    severity: RiskSeverity,
    limit: Decimal,
    actual: Decimal,
    unit: &str,
    message: &str,
) -> RiskViolation {
    RiskViolation {
        key: key.to_string(),
        label: label.to_string(),
        severity,
        limit,
        actual,
        unit: unit.to_string(),
        message: message.to_string(),
    }
}
