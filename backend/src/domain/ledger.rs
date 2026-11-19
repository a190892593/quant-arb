//! 统一资金账本（TDD §9）：账户、流水、余额、库存与对账。
//!
//! 原则：
//! - 流水不可覆盖，只能追加；余额是流水的累积结果，不单独维护「真值」。
//! - 每笔流水包含来源、去向、交易前后余额、费用、时间与关联 ID。
//! - 跨场所资产分开核算，合并净持仓与跨场所净敞口分别报告。
//! - 库存再平衡单独记账，不隐藏在策略利润里。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::types::{AccountKind, EntryKind, StrategyType, VenueKind};

/// 账本账户。一个账户 = 场所 + 资产 + 用途。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub account_id: String,
    pub kind: AccountKind,
    pub venue_kind: VenueKind,
    /// 场所标识：`dex-arb` / `cex-binance` / `flash` …
    pub venue: String,
    pub chain_id: Option<u64>,
    pub asset: String,
    pub label: String,
    /// 受保护账户（如闪电贷清算户）不允许被普通流水改动。
    pub protected: bool,
    pub created_at: DateTime<Utc>,
}

/// 一条账本流水（签名金额：正为流入账户，负为流出）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub entry_id: String,
    pub ts: DateTime<Utc>,
    pub account_id: String,
    pub kind: EntryKind,
    pub asset: String,
    pub amount: Decimal,
    /// 记账后余额（由引擎计算并固化，便于审计核对）。
    pub balance_after: Decimal,
    /// 该笔流水的 USDC 估值（按记账时价格）。
    pub usd_value: Decimal,
    pub price_usd: Decimal,
    pub strategy_type: Option<StrategyType>,
    pub opportunity_id: Option<String>,
    pub plan_id: Option<String>,
    pub execution_id: Option<String>,
    pub ref_id: Option<String>,
    pub memo: String,
}

/// 账户余额视图（由流水聚合）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountBalance {
    pub account_id: String,
    pub kind: AccountKind,
    pub venue: String,
    pub asset: String,
    pub balance: Decimal,
    pub available: Decimal,
    pub frozen: Decimal,
    pub pending: Decimal,
    pub usd_price: Decimal,
    pub usd_value: Decimal,
    pub updated_at: DateTime<Utc>,
}

/// 库存快照：按场所 + 资产记录可用/冻结/待结算。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventorySnapshot {
    pub ts: DateTime<Utc>,
    pub venue: String,
    pub venue_kind: VenueKind,
    pub asset: String,
    pub available: Decimal,
    pub frozen: Decimal,
    pub pending: Decimal,
    pub usd_price: Decimal,
    pub usd_value: Decimal,
}

/// 场所账户汇总（账户与库存页面）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VenueAccount {
    pub venue: String,
    pub name: String,
    pub kind: VenueKind,
    pub chain: Option<String>,
    pub state: String,
    pub usd_value: Decimal,
    pub assets: Vec<AssetHolding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetHolding {
    pub asset: String,
    pub qty: Decimal,
    pub usd_price: Decimal,
    pub usd_value: Decimal,
    pub available: Decimal,
    pub frozen: Decimal,
    pub pending: Decimal,
}

/// 合并持仓与跨场所敞口（TDD §9.3）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CombinedPosition {
    pub asset: String,
    pub cex_qty: Decimal,
    pub dex_qty: Decimal,
    pub total_qty: Decimal,
    pub usd_price: Decimal,
    pub usd_value: Decimal,
    /// 跨场所净敞口（CEX 与 DEX 数量差），用于库存再平衡判断。
    pub cross_venue_imbalance: Decimal,
}

/// 对账结果。任何不一致都必须停止对应策略（风控 §10）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reconciliation {
    pub ts: DateTime<Utc>,
    pub scope: String,
    pub expected: Decimal,
    pub actual: Decimal,
    pub diff: Decimal,
    pub ok: bool,
    pub notes: String,
}

impl Reconciliation {
    pub fn new(scope: &str, expected: Decimal, actual: Decimal, notes: &str) -> Self {
        let diff = actual - expected;
        Self {
            ts: Utc::now(),
            scope: scope.to_string(),
            expected,
            actual,
            diff,
            ok: diff.abs() <= Decimal::new(1, 6),
            notes: notes.to_string(),
        }
    }
}

/// 账本流水查询条件（API `GET /ledger`）。
#[derive(Clone, Debug, Default, Deserialize)]
pub struct LedgerQuery {
    pub account_id: Option<String>,
    pub asset: Option<String>,
    pub kind: Option<String>,
    pub strategy: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}
