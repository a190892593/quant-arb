//! 全局枚举。字符串表示即 API / DB / 前端契约，**不得随意改名**。

/// 声明一个「字符串即状态」的枚举：同时提供 serde、Display、FromStr 和 `as_str`。
///
/// 使用方式：
/// ```ignore
/// str_enum! {
///     #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
///     pub enum Side { Buy => "BUY", Sell => "SELL" }
/// }
/// ```
#[macro_export]
macro_rules! str_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident => $s:literal),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $(#[serde(rename = $s)] $variant,)*
        }

        impl $name {
            /// 全部变体，顺序与声明一致。
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            /// 契约字符串。
            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $s),*
                }
            }

            /// 从契约字符串解析。
            pub fn parse(s: &str) -> Option<Self> {
                match s {
                    $($s => Some($name::$variant),)*
                    _ => None,
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s).ok_or_else(|| format!("unknown {}: {}", stringify!($name), s))
            }
        }
    };
}

use serde::{Deserialize, Serialize};

str_enum! {
    /// 策略类型，与数据结构中的 `strategy_id` 一致。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum StrategyType {
        DexTriple => "dex_triple",
        DexCross => "dex_cross",
        CexDex => "cex_dex",
    }
}

impl StrategyType {
    /// 是否为同链原子策略（可进入链上 Executor）。
    pub fn is_atomic(&self) -> bool {
        matches!(self, StrategyType::DexTriple | StrategyType::DexCross)
    }

    /// 中文展示名。
    pub fn label(&self) -> &'static str {
        match self {
            StrategyType::DexTriple => "DEX 原子三角套利",
            StrategyType::DexCross => "DEX 原子跨池套利",
            StrategyType::CexDex => "CEX–DEX 非原子对冲",
        }
    }
}

str_enum! {
    /// 机会状态。只有 ACTIONABLE 才可能生成可执行计划。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum OppStatus {
        Actionable => "ACTIONABLE",
        Watch => "WATCH",
        Rejected => "REJECTED",
        Expired => "EXPIRED",
    }
}

str_enum! {
    /// 数据质量等级。INVALID 一律禁止生成可执行机会。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum DataQuality {
        Verified => "A",
        High => "B",
        Medium => "C",
        Low => "D",
        Invalid => "INVALID",
    }
}

impl DataQuality {
    /// 是否达到「可作为可执行机会依据」的门槛。
    pub fn is_executable(&self) -> bool {
        matches!(self, DataQuality::Verified | DataQuality::High)
    }

    /// 数值等级，越小越好，用于比较与评分。
    pub fn rank(&self) -> u8 {
        match self {
            DataQuality::Verified => 0,
            DataQuality::High => 1,
            DataQuality::Medium => 2,
            DataQuality::Low => 3,
            DataQuality::Invalid => 4,
        }
    }
}

str_enum! {
    /// 链准入等级（TDD §3.1）。只能逐级前进，且有验证记录。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum ChainStage {
        Discovered => "DISCOVERED",
        DataValidated => "DATA_VALIDATED",
        HistoricalReady => "HISTORICAL_READY",
        PaperReady => "PAPER_READY",
        LiveCandidate => "LIVE_CANDIDATE",
        LiveApproved => "LIVE_APPROVED",
    }
}

impl ChainStage {
    /// 是否允许在 Paper 环境扫描与模拟。
    pub fn paper_allowed(&self) -> bool {
        matches!(
            self,
            ChainStage::PaperReady | ChainStage::LiveCandidate | ChainStage::LiveApproved
        )
    }

    /// 是否允许真实执行（第一阶段恒为关闭）。
    pub fn live_allowed(&self) -> bool {
        matches!(self, ChainStage::LiveApproved)
    }
}

str_enum! {
    /// 风控参数等级（风控 §2）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum RiskSeverity {
        HardStop => "HARD_STOP",
        SoftBlock => "SOFT_BLOCK",
        Warning => "WARNING",
        PaperOnly => "PAPER_ONLY",
    }
}

str_enum! {
    /// 风控作用域（风控 §13）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum RiskScope {
        Global => "GLOBAL",
        Strategy => "STRATEGY",
        Chain => "CHAIN",
        Exchange => "EXCHANGE",
        Symbol => "SYMBOL",
        Token => "TOKEN",
    }
}

str_enum! {
    /// 参数方向的语义：上限（越小越安全）或下限（越大越安全）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum ParamDir {
        Max => "MAX",
        Min => "MIN",
    }
}

str_enum! {
    /// 审计模型等级（TDD §12.3）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum ModelLevel {
        M0 => "M0",
        M1 => "M1",
        M2 => "M2",
        M3 => "M3",
    }
}

str_enum! {
    /// CEX–DEX 方向。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum Direction {
        DexBuyCexSell => "DEX_BUY_CEX_SELL",
        CexBuyDexSell => "CEX_BUY_DEX_SELL",
    }
}

impl Direction {
    /// DEX 腿是买入 base 还是卖出 base。
    pub fn dex_is_buy(&self) -> bool {
        matches!(self, Direction::DexBuyCexSell)
    }
}

str_enum! {
    /// 双腿执行顺序（TDD §7.1 / §7.2）。Paper 必须分别统计。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum HedgeOrder {
        CexFirst => "CEX_FIRST",
        DexFirst => "DEX_FIRST",
    }
}

str_enum! {
    /// 统一执行状态机（TDD §7）。Paper 与未来 Live 共用。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum ExecutionState {
        Detected => "DETECTED",
        Validated => "VALIDATED",
        Planned => "PLANNED",
        HedgeSubmitted => "HEDGE_SUBMITTED",
        DexSubmitted => "DEX_SUBMITTED",
        PartiallyFilled => "PARTIALLY_FILLED",
        LegConfirmed => "LEG_CONFIRMED",
        SecondLegPending => "SECOND_LEG_PENDING",
        Hedged => "HEDGED",
        Reconciled => "RECONCILED",
        Rejected => "REJECTED",
        Expired => "EXPIRED",
        OrderStatusUnknown => "ORDER_STATUS_UNKNOWN",
        PartialHedge => "PARTIAL_HEDGE",
        DexReverted => "DEX_REVERTED",
        HedgeFailed => "HEDGE_FAILED",
        UnwindRequired => "UNWIND_REQUIRED",
        UnwindFailed => "UNWIND_FAILED",
        ManualRecoveryRequired => "MANUAL_RECOVERY_REQUIRED",
    }
}

impl ExecutionState {
    /// 终态：不再推进。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ExecutionState::Reconciled
                | ExecutionState::Rejected
                | ExecutionState::Expired
                | ExecutionState::UnwindFailed
                | ExecutionState::ManualRecoveryRequired
                | ExecutionState::HedgeFailed
                | ExecutionState::DexReverted
        )
    }

    /// 异常状态：必须产生风险事件。
    pub fn is_exception(&self) -> bool {
        matches!(
            self,
            ExecutionState::OrderStatusUnknown
                | ExecutionState::PartialHedge
                | ExecutionState::DexReverted
                | ExecutionState::HedgeFailed
                | ExecutionState::UnwindRequired
                | ExecutionState::UnwindFailed
                | ExecutionState::ManualRecoveryRequired
        )
    }

    /// 是否处于「第一腿已成立、第二腿未完成」的敞口状态。
    pub fn is_exposed(&self) -> bool {
        matches!(
            self,
            ExecutionState::PartiallyFilled
                | ExecutionState::LegConfirmed
                | ExecutionState::SecondLegPending
                | ExecutionState::PartialHedge
        )
    }
}

str_enum! {
    /// 单腿（CEX 订单）状态（风控 §7.1）。UNKNOWN 不等于未成交。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum OrderStatus {
        NotSubmitted => "NOT_SUBMITTED",
        Submitted => "SUBMITTED",
        Acknowledged => "ACKNOWLEDGED",
        PartiallyFilled => "PARTIALLY_FILLED",
        Filled => "FILLED",
        Canceled => "CANCELED",
        Rejected => "REJECTED",
        Unknown => "UNKNOWN",
    }
}

impl OrderStatus {
    /// 已确认成交量大于 0 的状态。
    pub fn has_fill(&self) -> bool {
        matches!(self, OrderStatus::PartiallyFilled | OrderStatus::Filled)
    }
}

str_enum! {
    /// 买卖方向。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum Side {
        Buy => "BUY",
        Sell => "SELL",
    }
}

impl Side {
    pub fn opposite(&self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

str_enum! {
    /// CEX 委托类型。第一阶段只允许带价格保护的 IOC / FOK / LIMIT。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum OrderType {
        Ioc => "IOC",
        Fok => "FOK",
        Limit => "LIMIT",
        Market => "MARKET",
    }
}

str_enum! {
    /// 池类型。只有经过适配验证的池类型可进入执行白名单。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum PoolKind {
        UniswapV2 => "UNISWAP_V2",
        UniswapV3 => "UNISWAP_V3",
        StableSwap => "STABLESWAP",
        WeightedPool => "WEIGHTED",
        Unknown => "UNKNOWN",
    }
}

impl PoolKind {
    /// V4 / Hook / 非标准池在验证前不得自动进入执行白名单。
    pub fn execution_allowed(&self) -> bool {
        !matches!(self, PoolKind::Unknown)
    }
}

str_enum! {
    /// 场所类型。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum VenueKind {
        Dex => "DEX",
        Cex => "CEX",
        Clearing => "CLEARING",
    }
}

str_enum! {
    /// 统一资金账本账户类型（TDD §9.1）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum AccountKind {
        DexWallet => "DEX_WALLET",
        CexSpot => "CEX_SPOT",
        FlashClearing => "FLASH_CLEARING",
        GasExpense => "GAS_EXPENSE",
        CexFee => "CEX_FEE",
        DexFee => "DEX_FEE",
        HedgeCompensation => "HEDGE_COMPENSATION",
        InventoryRebalance => "INVENTORY_REBALANCE",
        RealizedPnl => "REALIZED_PNL",
        UnrealizedPnl => "UNREALIZED_PNL",
    }
}

impl AccountKind {
    pub fn label(&self) -> &'static str {
        match self {
            AccountKind::DexWallet => "DEX 钱包",
            AccountKind::CexSpot => "CEX 现货",
            AccountKind::FlashClearing => "闪电贷清算",
            AccountKind::GasExpense => "Gas 支出",
            AccountKind::CexFee => "CEX 手续费",
            AccountKind::DexFee => "DEX 手续费",
            AccountKind::HedgeCompensation => "对冲补偿",
            AccountKind::InventoryRebalance => "库存再平衡",
            AccountKind::RealizedPnl => "已实现盈亏",
            AccountKind::UnrealizedPnl => "未实现盈亏",
        }
    }
}

str_enum! {
    /// 账本流水类型。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum EntryKind {
        Deposit => "DEPOSIT",
        Trade => "TRADE",
        Fee => "FEE",
        Gas => "GAS",
        FlashBorrow => "FLASH_BORROW",
        FlashRepay => "FLASH_REPAY",
        Transfer => "TRANSFER",
        Pnl => "PNL",
        Compensation => "COMPENSATION",
        Rebalance => "REBALANCE",
        Adjustment => "ADJUSTMENT",
    }
}

str_enum! {
    /// 风险事件类型（风控 §10 熔断规则）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum RiskEventKind {
        DailyLoss => "DAILY_LOSS",
        StrategyDailyLoss => "STRATEGY_DAILY_LOSS",
        Drawdown => "DRAWDOWN",
        DexReverts => "DEX_REVERTS",
        HedgeAnomaly => "HEDGE_ANOMALY",
        RpcUnhealthy => "RPC_UNHEALTHY",
        BookInvalid => "BOOK_INVALID",
        OrderUnknown => "ORDER_UNKNOWN",
        LedgerMismatch => "LEDGER_MISMATCH",
        TokenRisk => "TOKEN_RISK",
        PoolRisk => "POOL_RISK",
        AdapterRisk => "ADAPTER_RISK",
        ManualPause => "MANUAL_PAUSE",
        DataStale => "DATA_STALE",
        ExposureBreach => "EXPOSURE_BREACH",
        ParameterChange => "PARAMETER_CHANGE",
        Recovery => "RECOVERY",
    }
}

str_enum! {
    /// 审计结论（TDD §12.4）。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum AuditVerdict {
        Pass => "PASS",
        Watch => "WATCH",
        Reject => "REJECT",
        Paper => "PAPER",
    }
}

str_enum! {
    /// 审计任务状态。
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
    pub enum JobState {
        Queued => "QUEUED",
        Running => "RUNNING",
        Completed => "COMPLETED",
        Failed => "FAILED",
        Canceled => "CANCELED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_strings_are_stable() {
        assert_eq!(StrategyType::DexTriple.as_str(), "dex_triple");
        assert_eq!(StrategyType::DexCross.as_str(), "dex_cross");
        assert_eq!(StrategyType::CexDex.as_str(), "cex_dex");
        assert_eq!(ExecutionState::OrderStatusUnknown.as_str(), "ORDER_STATUS_UNKNOWN");
        assert_eq!(AccountKind::FlashClearing.as_str(), "FLASH_CLEARING");
    }

    #[test]
    fn roundtrip_all_variants() {
        for e in ExecutionState::ALL {
            assert_eq!(ExecutionState::parse(e.as_str()), Some(*e));
        }
        for e in RiskEventKind::ALL {
            assert_eq!(RiskEventKind::parse(e.as_str()), Some(*e));
        }
        for e in ModelLevel::ALL {
            assert_eq!(ModelLevel::parse(e.as_str()), Some(*e));
        }
    }

    #[test]
    fn serde_uses_contract_strings() {
        let j = serde_json::to_string(&StrategyType::CexDex).unwrap();
        assert_eq!(j, "\"cex_dex\"");
        let s: ExecutionState = serde_json::from_str("\"PARTIAL_HEDGE\"").unwrap();
        assert_eq!(s, ExecutionState::PartialHedge);
    }

    #[test]
    fn atomicity_semantics() {
        assert!(StrategyType::DexTriple.is_atomic());
        assert!(!StrategyType::CexDex.is_atomic());
        assert!(ExecutionState::PartialHedge.is_exposed());
        assert!(ExecutionState::Reconciled.is_terminal());
        assert!(!DataQuality::Low.is_executable());
        assert!(DataQuality::Verified.is_executable());
    }
}
