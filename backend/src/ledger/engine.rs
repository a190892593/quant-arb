//! 统一资金账本引擎（TDD §9、PRD §5.7）。
//!
//! # 记账口径（全模块唯一口径，报表与绩效均以此为准）
//!
//! - 流水金额是**签名金额**：正数流入账户、负数流出账户；`balance_after` 由引擎计算并固化。
//! - **资产类账户**（`DEX_WALLET` / `CEX_SPOT` / `FLASH_CLEARING`）：余额即持仓，不得为负，
//!   透支返回 [`LedgerError::InsufficientBalance`]。权益 = 资产类账户余额 × 价格之和。
//! - **成本类账户**（`GAS_EXPENSE` / `CEX_FEE` / `DEX_FEE` / `HEDGE_COMPENSATION` /
//!   `INVENTORY_REBALANCE`）：以**正数累计成本**，对应资金从资产类账户以负数流出（同一次
//!   `post_batch` 成对记账）。这类账户允许正负累积（退款、校正），但**不参与权益计算**。
//! - `REALIZED_PNL`：只记录**毛**已实现盈亏（策略点差收益，不含任何成本）；同时由资产账户
//!   记录真实资金变化，两者互为镜像，便于逐笔归因。
//! - `UNREALIZED_PNL`：标记价重估产生的未实现盈亏，同样不计入权益（权益由资产账户余额 ×
//!   价格给出）。
//!
//! 因此报表的净盈亏口径是：
//! `净盈亏 = 已实现毛盈亏 + 未实现盈亏 − 费用 − gas − 补偿 − 再平衡`，每一项只扣一次
//! （成本已单独记账，`REALIZED_PNL` 里不能再预先扣费）。
//!
//! # 其他约定
//!
//! - 账户 ID 形如 `场所:资产:账户类型`（如 `dex-arb:USDC:DEX_WALLET`），由
//!   [`LedgerEngine::ensure_account`] 确定性生成，便于对账与持久化。
//! - 批量过账是原子的：先整批校验（暂存余额副本），任一失败则整批不落账。
//! - 价格表是显式状态（[`LedgerEngine::set_price`]）。首次出现的新资产会用该笔流水的
//!   `price_usd` 补登记一次，保证 `usd_value` 与权益口径一致；之后只由 `set_price` 更新。

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::domain::ledger::{Account, AccountBalance, LedgerEntry, LedgerQuery, Reconciliation};
use crate::domain::types::{AccountKind, EntryKind, StrategyType, VenueKind};
use crate::domain::{new_id, now};

/// 资产类账户：余额即持仓，不得为负，计入权益。
pub fn is_asset_kind(kind: AccountKind) -> bool {
    matches!(
        kind,
        AccountKind::DexWallet | AccountKind::CexSpot | AccountKind::FlashClearing
    )
}

/// 成本类账户：正数累计成本（允许负累积），不计入权益。
pub fn is_cost_kind(kind: AccountKind) -> bool {
    matches!(
        kind,
        AccountKind::GasExpense
            | AccountKind::CexFee
            | AccountKind::DexFee
            | AccountKind::HedgeCompensation
            | AccountKind::InventoryRebalance
    )
}

/// 标准账户的默认「场所 / 场所类型 / 资产」。启动时 10 类账户都按此创建。
fn default_venue(kind: AccountKind) -> (&'static str, VenueKind, &'static str) {
    match kind {
        AccountKind::DexWallet => ("dex-arb", VenueKind::Dex, "USDC"),
        AccountKind::CexSpot => ("cex-binance", VenueKind::Cex, "USDT"),
        AccountKind::FlashClearing => ("flash", VenueKind::Clearing, "USDC"),
        AccountKind::GasExpense => ("dex-arb", VenueKind::Dex, "USDC"),
        AccountKind::CexFee => ("cex-binance", VenueKind::Cex, "USDT"),
        AccountKind::DexFee => ("dex-arb", VenueKind::Dex, "USDC"),
        AccountKind::HedgeCompensation => ("cex-binance", VenueKind::Cex, "USDT"),
        AccountKind::InventoryRebalance => ("dex-arb", VenueKind::Dex, "USDC"),
        AccountKind::RealizedPnl => ("global", VenueKind::Clearing, "USDC"),
        AccountKind::UnrealizedPnl => ("global", VenueKind::Clearing, "USDC"),
    }
}

fn chain_for(venue_kind: VenueKind) -> Option<u64> {
    // 第一阶段只有 DEX 腿绑定到具体链；CEX 与全局账户没有链。
    match venue_kind {
        VenueKind::Dex => Some(1),
        _ => None,
    }
}

/// 确定性账户 ID：`场所:资产:账户类型`。
fn account_id_for(venue: &str, asset: &str, kind: AccountKind) -> String {
    format!("{venue}:{asset}:{}", kind.as_str())
}

/// 一次过账请求。
#[derive(Clone, Debug)]
pub struct PostRequest {
    pub account_id: String,
    pub kind: EntryKind,
    pub asset: String,
    /// 签名金额：正数流入账户，负数流出账户。
    pub amount: Decimal,
    pub price_usd: Decimal,
    pub strategy_type: Option<StrategyType>,
    pub opportunity_id: Option<String>,
    pub plan_id: Option<String>,
    pub execution_id: Option<String>,
    pub ref_id: Option<String>,
    pub memo: String,
}

impl PostRequest {
    /// 构造一条最小请求（无策略/执行关联），供内部与测试使用。
    pub fn new(
        account_id: impl Into<String>,
        kind: EntryKind,
        asset: impl Into<String>,
        amount: Decimal,
        price_usd: Decimal,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            kind,
            asset: asset.into(),
            amount,
            price_usd,
            strategy_type: None,
            opportunity_id: None,
            plan_id: None,
            execution_id: None,
            ref_id: None,
            memo: String::new(),
        }
    }
}

/// 账本错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerError {
    /// 账户不存在。
    UnknownAccount(String),
    /// 流水资产与账户资产不一致。
    AssetMismatch {
        account: String,
        expected: String,
        got: String,
    },
    /// 资产类账户余额不足（不允许透支）。
    InsufficientBalance {
        account: String,
        /// 记账前余额。
        balance: Decimal,
        /// 请求的签名金额。
        requested: Decimal,
    },
    /// 受保护账户（闪电贷清算户）只允许闪电贷借款/偿还流水。
    ProtectedAccount(String),
    /// 金额为 0：签名金额必须非 0（负数表示流出，是合法金额）。
    NegativeAmount,
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::UnknownAccount(id) => write!(f, "未知账户：{id}"),
            LedgerError::AssetMismatch {
                account,
                expected,
                got,
            } => write!(
                f,
                "账户 {account} 的资产为 {expected}，流水资产为 {got}，不允许跨资产记账"
            ),
            LedgerError::InsufficientBalance {
                account,
                balance,
                requested,
            } => write!(
                f,
                "账户 {account} 余额不足：当前 {balance}，请求 {requested}（资产类账户不允许透支）"
            ),
            LedgerError::ProtectedAccount(id) => write!(
                f,
                "账户 {id} 是受保护账户，只允许闪电贷借款（FLASH_BORROW）与偿还（FLASH_REPAY）流水"
            ),
            LedgerError::NegativeAmount => {
                write!(f, "流水金额非法：金额不得为 0，负数表示流出账户")
            }
        }
    }
}

impl std::error::Error for LedgerError {}

/// 一笔已结算交易的统计记录（不做资金变动，资金变动必须走 [`LedgerEngine::post`]）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeStat {
    pub execution_id: String,
    pub strategy_type: StrategyType,
    /// 逐笔净盈亏（已扣该笔成本）。
    pub net_pnl: Decimal,
    /// 逐笔毛盈亏（未扣成本）。
    pub gross_pnl: Decimal,
    pub cost: Decimal,
    pub filled_ratio: Decimal,
    pub reverted: bool,
    pub exposure_ms: i64,
    pub notional: Decimal,
    pub closed_at: DateTime<Utc>,
}

/// 完整账本快照（持久化 / 导出 / 恢复）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerSnapshot {
    pub accounts: Vec<Account>,
    pub entries: Vec<LedgerEntry>,
    pub balances: BTreeMap<String, Decimal>,
    pub prices: BTreeMap<String, Decimal>,
    pub trades: Vec<TradeStat>,
    pub start_equity: Decimal,
}

/// 统一资金账本引擎。
///
/// 内部状态全部来自快照字段，保证 [`LedgerEngine::export`] / [`LedgerEngine::restore`] 无损往返。
#[derive(Clone, Debug)]
pub struct LedgerEngine {
    /// 账户表（顺序即创建顺序，输出稳定）。
    accounts: Vec<Account>,
    /// 追加式流水，只增不改。
    entries: Vec<LedgerEntry>,
    /// 账户余额聚合（流水的累积结果）。
    balances: BTreeMap<String, Decimal>,
    /// 资产价格表（USDC / USDT 恒为 1.0）。
    prices: BTreeMap<String, Decimal>,
    /// 已结算交易统计（绩效原始数据）。
    trades: Vec<TradeStat>,
    /// 期初权益（Paper 重置基准）。
    start_equity: Decimal,
}

impl LedgerEngine {
    /// 以 `start_equity` 建立标准账户：DEX 钱包与 CEX 现货各 50%，资产 USDC/USDT，价格 1.0。
    ///
    /// 10 类账户全部创建（其余 8 类余额为 0），并留下两笔 `DEPOSIT` 期初流水，保证资金
    /// 来源可追溯。
    pub fn new(start_equity: Decimal) -> Self {
        let mut engine = Self {
            accounts: Vec::new(),
            entries: Vec::new(),
            balances: BTreeMap::new(),
            prices: BTreeMap::new(),
            trades: Vec::new(),
            start_equity,
        };
        engine.prices.insert("USDC".to_string(), Decimal::ONE);
        engine.prices.insert("USDT".to_string(), Decimal::ONE);

        // 先建齐 10 类账户，再注入期初资金，流水顺序稳定。
        let mut opening: Vec<(String, &'static str)> = Vec::new();
        for &kind in AccountKind::ALL {
            let (venue, venue_kind, asset) = default_venue(kind);
            let account_id = engine.ensure_account(venue, venue_kind, asset, kind);
            if matches!(kind, AccountKind::DexWallet | AccountKind::CexSpot) {
                opening.push((account_id, asset));
            }
        }

        let half = start_equity / Decimal::from(2);
        if half > Decimal::ZERO {
            for (account_id, asset) in opening {
                engine
                    .post(PostRequest {
                        account_id,
                        kind: EntryKind::Deposit,
                        asset: asset.to_string(),
                        amount: half,
                        price_usd: Decimal::ONE,
                        strategy_type: None,
                        opportunity_id: None,
                        plan_id: None,
                        execution_id: None,
                        ref_id: Some("opening".to_string()),
                        memo: "Paper 初始资金注入（DEX 钱包 : CEX 现货 = 50 : 50）".to_string(),
                    })
                    .expect("期初资金注入不可失败");
            }
        }
        engine
    }

    /// 全部账户（创建顺序）。
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// 按账户 ID 查账户。
    pub fn account(&self, account_id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.account_id == account_id)
    }

    /// 按 场所 + 资产 + 类型 查账户。
    pub fn find_account(&self, venue: &str, asset: &str, kind: AccountKind) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.venue == venue && a.asset == asset && a.kind == kind)
    }

    /// 确保账户存在（不存在则创建），返回 account_id。
    pub fn ensure_account(
        &mut self,
        venue: &str,
        venue_kind: VenueKind,
        asset: &str,
        kind: AccountKind,
    ) -> String {
        if let Some(existing) = self.find_account(venue, asset, kind) {
            return existing.account_id.clone();
        }
        let account_id = account_id_for(venue, asset, kind);
        let account = Account {
            account_id: account_id.clone(),
            kind,
            venue_kind,
            venue: venue.to_string(),
            chain_id: chain_for(venue_kind),
            asset: asset.to_string(),
            label: kind.label().to_string(),
            // 闪电贷清算户是受保护账户：普通流水不得改动其本金。
            protected: matches!(kind, AccountKind::FlashClearing),
            created_at: now(),
        };
        self.accounts.push(account);
        self.balances
            .entry(account_id.clone())
            .or_insert(Decimal::ZERO);
        account_id
    }

    /// 记账：校验 → 计算 `balance_after` → 追加流水并更新余额聚合。
    pub fn post(&mut self, req: PostRequest) -> Result<LedgerEntry, LedgerError> {
        let mut entries = self.post_batch(vec![req])?;
        Ok(entries.remove(0))
    }

    /// 原子批量记账：任一流水校验失败则整批不落账。
    pub fn post_batch(&mut self, reqs: Vec<PostRequest>) -> Result<Vec<LedgerEntry>, LedgerError> {
        // 阶段一：在余额副本上逐笔校验并计算 balance_after，失败即返回，self 完全不变。
        let mut staged = self.balances.clone();
        let mut entries: Vec<LedgerEntry> = Vec::with_capacity(reqs.len());
        for req in &reqs {
            let entry = self.build_entry(req, &staged)?;
            staged.insert(entry.account_id.clone(), entry.balance_after);
            entries.push(entry);
        }

        // 阶段二：提交。只为从未登记过的资产补一次价格，之后价格只由 set_price 更新。
        for entry in &entries {
            if entry.price_usd > Decimal::ZERO {
                self.prices
                    .entry(entry.asset.clone())
                    .or_insert(entry.price_usd);
            }
        }
        self.balances = staged;
        let out = entries.clone();
        self.entries.extend(entries);
        Ok(out)
    }

    /// 校验单笔请求并构造流水（不修改任何状态）。
    fn build_entry(
        &self,
        req: &PostRequest,
        staged: &BTreeMap<String, Decimal>,
    ) -> Result<LedgerEntry, LedgerError> {
        let account = self
            .account(&req.account_id)
            .ok_or_else(|| LedgerError::UnknownAccount(req.account_id.clone()))?;
        if account.asset != req.asset {
            return Err(LedgerError::AssetMismatch {
                account: account.account_id.clone(),
                expected: account.asset.clone(),
                got: req.asset.clone(),
            });
        }
        if account.protected && !matches!(req.kind, EntryKind::FlashBorrow | EntryKind::FlashRepay)
        {
            return Err(LedgerError::ProtectedAccount(account.account_id.clone()));
        }
        if req.amount.is_zero() {
            return Err(LedgerError::NegativeAmount);
        }
        let current = staged
            .get(&account.account_id)
            .copied()
            .unwrap_or_else(|| self.balance(&account.account_id));
        let balance_after = current + req.amount;
        if is_asset_kind(account.kind) && balance_after < Decimal::ZERO {
            return Err(LedgerError::InsufficientBalance {
                account: account.account_id.clone(),
                balance: current,
                requested: req.amount,
            });
        }
        Ok(LedgerEntry {
            entry_id: new_id("le"),
            ts: now(),
            account_id: account.account_id.clone(),
            kind: req.kind,
            asset: req.asset.clone(),
            amount: req.amount,
            balance_after,
            usd_value: req.amount * req.price_usd,
            price_usd: req.price_usd,
            strategy_type: req.strategy_type,
            opportunity_id: req.opportunity_id.clone(),
            plan_id: req.plan_id.clone(),
            execution_id: req.execution_id.clone(),
            ref_id: req.ref_id.clone(),
            memo: req.memo.clone(),
        })
    }

    /// 账户余额（未知账户按 0 处理，便于对账）。
    pub fn balance(&self, account_id: &str) -> Decimal {
        self.balances
            .get(account_id)
            .copied()
            .unwrap_or(Decimal::ZERO)
    }

    /// 全部账户的余额视图（按账户创建顺序）。
    pub fn balances(&self) -> Vec<AccountBalance> {
        let mut last_ts: BTreeMap<&str, DateTime<Utc>> = BTreeMap::new();
        for entry in &self.entries {
            last_ts.insert(entry.account_id.as_str(), entry.ts);
        }
        self.accounts
            .iter()
            .map(|account| {
                let balance = self.balance(&account.account_id);
                let price = self.price(&account.asset);
                AccountBalance {
                    account_id: account.account_id.clone(),
                    kind: account.kind,
                    venue: account.venue.clone(),
                    asset: account.asset.clone(),
                    balance,
                    // Paper 阶段不冻结资金：可用余额 = 账面余额。
                    available: balance,
                    frozen: Decimal::ZERO,
                    pending: Decimal::ZERO,
                    usd_price: price,
                    usd_value: balance * price,
                    updated_at: last_ts
                        .get(account.account_id.as_str())
                        .copied()
                        .unwrap_or(account.created_at),
                }
            })
            .collect()
    }

    /// 全部流水（追加顺序）。
    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// 按条件查询流水；`limit` 取**最近** N 条（保持时间顺序）。
    pub fn query(&self, q: &LedgerQuery) -> Vec<LedgerEntry> {
        let mut out: Vec<LedgerEntry> = self
            .entries
            .iter()
            .filter(|entry| {
                if let Some(account_id) = &q.account_id {
                    if &entry.account_id != account_id {
                        return false;
                    }
                }
                if let Some(asset) = &q.asset {
                    if !entry.asset.eq_ignore_ascii_case(asset) {
                        return false;
                    }
                }
                if let Some(kind) = &q.kind {
                    if !entry.kind.as_str().eq_ignore_ascii_case(kind) {
                        return false;
                    }
                }
                if let Some(strategy) = &q.strategy {
                    let matched = entry
                        .strategy_type
                        .is_some_and(|s| s.as_str().eq_ignore_ascii_case(strategy));
                    if !matched {
                        return false;
                    }
                }
                if let Some(since) = q.since {
                    if entry.ts < since {
                        return false;
                    }
                }
                if let Some(until) = q.until {
                    if entry.ts > until {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        if let Some(limit) = q.limit {
            if limit == 0 {
                out.clear();
            } else if out.len() > limit {
                out = out.split_off(out.len() - limit);
            }
        }
        out
    }

    /// 设置资产价格（负值按 0 处理）。价格是权益计算的唯一外部输入。
    pub fn set_price(&mut self, asset: &str, price_usd: Decimal) {
        self.prices
            .insert(asset.to_string(), price_usd.max(Decimal::ZERO));
    }

    /// 资产价格；未登记的资产按 0 估值（必须显式 `set_price`）。
    pub fn price(&self, asset: &str) -> Decimal {
        self.prices.get(asset).copied().unwrap_or(Decimal::ZERO)
    }

    /// 期初权益（Paper 重置基准）。
    pub fn start_equity(&self) -> Decimal {
        self.start_equity
    }

    /// 资产类账户总权益 = Σ 余额 × 价格。成本/盈亏账户一律不计入。
    pub fn total_equity(&self) -> Decimal {
        self.accounts
            .iter()
            .filter(|a| is_asset_kind(a.kind))
            .map(|a| self.balance(&a.account_id) * self.price(&a.asset))
            .sum()
    }

    /// 费用/成本聚合（均为正数）。
    pub fn total_fees(&self) -> Decimal {
        self.sum_kinds(&[AccountKind::CexFee, AccountKind::DexFee])
            .max(Decimal::ZERO)
    }

    /// Gas 支出合计（正数）。
    pub fn total_gas(&self) -> Decimal {
        self.sum_kinds(&[AccountKind::GasExpense])
            .max(Decimal::ZERO)
    }

    /// 对冲补偿合计（正数）。
    pub fn total_compensation(&self) -> Decimal {
        self.sum_kinds(&[AccountKind::HedgeCompensation])
            .max(Decimal::ZERO)
    }

    /// 库存再平衡成本合计（正数）。
    pub fn total_rebalance(&self) -> Decimal {
        self.sum_kinds(&[AccountKind::InventoryRebalance])
            .max(Decimal::ZERO)
    }

    /// 已实现盈亏账户净额（可正可负；口径为**毛**盈亏，不含成本）。
    pub fn realized_pnl(&self) -> Decimal {
        self.sum_kinds(&[AccountKind::RealizedPnl])
    }

    /// 某策略的已实现盈亏（按流水的策略归因求和）。
    pub fn strategy_realized_pnl(&self, strategy: StrategyType) -> Decimal {
        self.entries
            .iter()
            .filter(|e| e.strategy_type == Some(strategy))
            .filter(|e| self.kind_of(&e.account_id) == Some(AccountKind::RealizedPnl))
            .map(|e| e.amount)
            .sum()
    }

    /// 某策略已发生的成本合计（费用 + gas + 补偿 + 再平衡，按流水归一化到 USDC）。
    pub fn strategy_costs(&self, strategy: StrategyType) -> Decimal {
        self.entries
            .iter()
            .filter(|e| e.strategy_type == Some(strategy))
            .filter(|e| self.kind_of(&e.account_id).is_some_and(is_cost_kind))
            .map(|e| e.usd_value)
            .sum::<Decimal>()
            .max(Decimal::ZERO)
    }

    /// 按账户对比「期望余额」与「实际余额」，返回不一致项（风控 §10 ledger mismatch）。
    pub fn reconcile(&self, expected: &[(String, Decimal)]) -> Vec<Reconciliation> {
        expected
            .iter()
            .map(|(account_id, expected_balance)| {
                let actual = self.balance(account_id);
                let mut rec = Reconciliation::new(account_id, *expected_balance, actual, "");
                rec.notes = if self.account(account_id).is_none() {
                    "账户不存在：实际余额按 0 处理，期望值无法核对（风控 §10 LEDGER_MISMATCH）"
                        .to_string()
                } else if !rec.ok {
                    "账本余额与期望不一致，必须停止对应策略并核对（风控 §10 LEDGER_MISMATCH）"
                        .to_string()
                } else {
                    "账本余额与期望一致".to_string()
                };
                rec
            })
            .collect()
    }

    /// 记录一次已结算交易的统计（供 performance 使用）。不做资金变动。
    pub fn record_trade(&mut self, stat: TradeStat) {
        self.trades.push(stat);
    }

    /// 全部交易统计（结算顺序）。
    pub fn trade_stats(&self) -> &[TradeStat] {
        &self.trades
    }

    /// 重置为初始状态（Paper 重置）。
    pub fn reset(&mut self, start_equity: Decimal) {
        *self = Self::new(start_equity);
    }

    /// 导出完整快照（用于持久化）。
    pub fn export(&self) -> LedgerSnapshot {
        LedgerSnapshot {
            accounts: self.accounts.clone(),
            entries: self.entries.clone(),
            balances: self.balances.clone(),
            prices: self.prices.clone(),
            trades: self.trades.clone(),
            start_equity: self.start_equity,
        }
    }

    /// 从快照恢复；缺失的账户余额按 0 补齐，账户 ID 保持不变。
    pub fn restore(snapshot: LedgerSnapshot) -> Self {
        let mut balances = snapshot.balances;
        for account in &snapshot.accounts {
            balances
                .entry(account.account_id.clone())
                .or_insert(Decimal::ZERO);
        }
        Self {
            accounts: snapshot.accounts,
            entries: snapshot.entries,
            balances,
            prices: snapshot.prices,
            trades: snapshot.trades,
            start_equity: snapshot.start_equity,
        }
    }

    /// 账户类型（不存在返回 None）。
    fn kind_of(&self, account_id: &str) -> Option<AccountKind> {
        self.account(account_id).map(|a| a.kind)
    }

    /// 指定类型账户的余额合计。
    fn sum_kinds(&self, kinds: &[AccountKind]) -> Decimal {
        self.accounts
            .iter()
            .filter(|a| kinds.contains(&a.kind))
            .map(|a| self.balance(&a.account_id))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dec;

    fn engine() -> LedgerEngine {
        LedgerEngine::new(dec("5000"))
    }

    fn account_of(engine: &LedgerEngine, kind: AccountKind) -> String {
        engine
            .accounts()
            .iter()
            .find(|a| a.kind == kind)
            .unwrap_or_else(|| panic!("缺少账户类型 {kind}"))
            .account_id
            .clone()
    }

    fn req(
        account_id: &str,
        kind: EntryKind,
        asset: &str,
        amount: &str,
        price: &str,
    ) -> PostRequest {
        PostRequest::new(account_id, kind, asset, dec(amount), dec(price))
    }

    #[test]
    fn new_creates_all_account_kinds_with_5050_equity() {
        let engine = engine();
        assert_eq!(AccountKind::ALL.len(), 10);
        for kind in AccountKind::ALL {
            assert!(
                engine.accounts().iter().any(|a| a.kind == *kind),
                "缺少账户类型 {kind}"
            );
        }
        assert_eq!(engine.start_equity(), dec("5000"));
        assert_eq!(engine.total_equity(), dec("5000"));

        let dex = account_of(&engine, AccountKind::DexWallet);
        let cex = account_of(&engine, AccountKind::CexSpot);
        assert_eq!(engine.balance(&dex), dec("2500"));
        assert_eq!(engine.balance(&cex), dec("2500"));
        assert_eq!(engine.account(&dex).unwrap().asset, "USDC");
        assert_eq!(engine.account(&cex).unwrap().asset, "USDT");
        assert_eq!(engine.price("USDC"), dec("1"));
        assert_eq!(engine.price("USDT"), dec("1"));
        // 期初资金留下两笔可审计的 DEPOSIT 流水。
        assert_eq!(
            engine
                .entries()
                .iter()
                .filter(|e| e.kind == EntryKind::Deposit)
                .count(),
            2
        );
        // 余额视图覆盖全部账户。
        assert_eq!(engine.balances().len(), 10);
    }

    #[test]
    fn post_updates_balance_and_balance_after() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let before = engine.entries().len();
        let entry = engine
            .post(req(&dex, EntryKind::Trade, "USDC", "-100.50", "1"))
            .unwrap();
        assert_eq!(entry.amount, dec("-100.50"));
        assert_eq!(entry.price_usd, dec("1"));
        assert_eq!(entry.usd_value, dec("-100.50"));
        assert_eq!(entry.balance_after, dec("2399.50"));
        assert_eq!(engine.balance(&dex), dec("2399.50"));
        assert_eq!(engine.entries().len(), before + 1);
        assert_eq!(engine.total_equity(), dec("4899.50"));
    }

    #[test]
    fn overdraft_is_rejected_without_side_effects() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let entries = engine.entries().len();
        let err = engine
            .post(req(&dex, EntryKind::Transfer, "USDC", "-2500.01", "1"))
            .unwrap_err();
        assert_eq!(
            err,
            LedgerError::InsufficientBalance {
                account: dex.clone(),
                balance: dec("2500"),
                requested: dec("-2500.01"),
            }
        );
        assert_eq!(engine.entries().len(), entries);
        assert_eq!(engine.balance(&dex), dec("2500"));
    }

    #[test]
    fn batch_post_is_atomic() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let gas = account_of(&engine, AccountKind::GasExpense);
        let entries = engine.entries().len();
        let err = engine
            .post_batch(vec![
                req(&dex, EntryKind::Gas, "USDC", "-5", "1"),
                req(&gas, EntryKind::Gas, "USDC", "5", "1"),
                req(&dex, EntryKind::Gas, "USDC", "-9999", "1"),
            ])
            .unwrap_err();
        assert!(matches!(err, LedgerError::InsufficientBalance { .. }));
        // 任一失败则整批不落账。
        assert_eq!(engine.entries().len(), entries);
        assert_eq!(engine.balance(&dex), dec("2500"));
        assert_eq!(engine.balance(&gas), Decimal::ZERO);

        let ok = engine
            .post_batch(vec![
                req(&dex, EntryKind::Gas, "USDC", "-5", "1"),
                req(&gas, EntryKind::Gas, "USDC", "5", "1"),
            ])
            .unwrap();
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[0].balance_after, dec("2495"));
        assert_eq!(ok[1].balance_after, dec("5"));
        assert_eq!(engine.balance(&gas), dec("5"));
    }

    #[test]
    fn rejects_unknown_account_asset_mismatch_zero_and_protected() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        assert_eq!(
            engine
                .post(req("no-such-account", EntryKind::Trade, "USDC", "1", "1"))
                .unwrap_err(),
            LedgerError::UnknownAccount("no-such-account".to_string())
        );
        assert_eq!(
            engine
                .post(req(&dex, EntryKind::Trade, "USDT", "1", "1"))
                .unwrap_err(),
            LedgerError::AssetMismatch {
                account: dex.clone(),
                expected: "USDC".to_string(),
                got: "USDT".to_string(),
            }
        );
        assert_eq!(
            engine
                .post(req(&dex, EntryKind::Trade, "USDC", "0", "1"))
                .unwrap_err(),
            LedgerError::NegativeAmount
        );

        // 闪电贷清算户受保护：普通流水被拒，借款/偿还允许。
        let flash = account_of(&engine, AccountKind::FlashClearing);
        assert!(engine.account(&flash).unwrap().protected);
        assert_eq!(
            engine
                .post(req(&flash, EntryKind::Trade, "USDC", "1", "1"))
                .unwrap_err(),
            LedgerError::ProtectedAccount(flash.clone())
        );
        engine
            .post(req(&flash, EntryKind::FlashBorrow, "USDC", "1000", "1"))
            .unwrap();
        assert_eq!(engine.balance(&flash), dec("1000"));
        engine
            .post(req(&flash, EntryKind::FlashRepay, "USDC", "-1000", "1"))
            .unwrap();
        assert_eq!(engine.balance(&flash), Decimal::ZERO);
    }

    #[test]
    fn expense_accounts_accumulate_and_do_not_change_equity() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let cex = account_of(&engine, AccountKind::CexSpot);
        let gas = account_of(&engine, AccountKind::GasExpense);
        let fee = account_of(&engine, AccountKind::CexFee);
        let rp = account_of(&engine, AccountKind::RealizedPnl);
        engine
            .post_batch(vec![
                req(&dex, EntryKind::Gas, "USDC", "-7", "1"),
                req(&gas, EntryKind::Gas, "USDC", "7", "1"),
                req(&cex, EntryKind::Fee, "USDT", "-3", "1"),
                req(&fee, EntryKind::Fee, "USDT", "3", "1"),
                req(&rp, EntryKind::Pnl, "USDC", "5", "1"),
            ])
            .unwrap();

        assert_eq!(engine.balance(&gas), dec("7"));
        assert_eq!(engine.balance(&fee), dec("3"));
        // 权益只由资产类账户决定：成本 7 + 3 已流出。
        assert_eq!(engine.total_equity(), dec("4990"));
        // 成本/盈亏账户本身不计入权益。
        assert_eq!(engine.total_gas(), dec("7"));
        assert_eq!(engine.total_fees(), dec("3"));
        assert_eq!(engine.realized_pnl(), dec("5"));

        // 成本账户允许负累积（退款/校正），且不影响权益。
        engine
            .post(req(&gas, EntryKind::Adjustment, "USDC", "-2", "1"))
            .unwrap();
        assert_eq!(engine.balance(&gas), dec("5"));
        assert_eq!(engine.total_equity(), dec("4990"));
    }

    #[test]
    fn paired_settlement_reduces_equity_by_cost_sum() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let cex = account_of(&engine, AccountKind::CexSpot);
        let gas = account_of(&engine, AccountKind::GasExpense);
        let cex_fee = account_of(&engine, AccountKind::CexFee);
        let dex_fee = account_of(&engine, AccountKind::DexFee);
        let comp = account_of(&engine, AccountKind::HedgeCompensation);
        let reb = account_of(&engine, AccountKind::InventoryRebalance);
        engine
            .post_batch(vec![
                req(&dex, EntryKind::Gas, "USDC", "-2", "1"),
                req(&gas, EntryKind::Gas, "USDC", "2", "1"),
                req(&cex, EntryKind::Fee, "USDT", "-1.5", "1"),
                req(&cex_fee, EntryKind::Fee, "USDT", "1.5", "1"),
                req(&dex, EntryKind::Fee, "USDC", "-0.5", "1"),
                req(&dex_fee, EntryKind::Fee, "USDC", "0.5", "1"),
                req(&cex, EntryKind::Compensation, "USDT", "-1", "1"),
                req(&comp, EntryKind::Compensation, "USDT", "1", "1"),
                req(&dex, EntryKind::Rebalance, "USDC", "-0.25", "1"),
                req(&reb, EntryKind::Rebalance, "USDC", "0.25", "1"),
            ])
            .unwrap();

        let cost = engine.total_fees()
            + engine.total_gas()
            + engine.total_compensation()
            + engine.total_rebalance();
        assert_eq!(cost, dec("5.25"));
        // 成对结算后权益减少量恰好等于成本分项之和。
        assert_eq!(engine.total_equity(), dec("5000") - cost);
    }

    #[test]
    fn price_changes_move_equity_for_non_stable_assets() {
        let mut engine = engine();
        let eth = engine.ensure_account("dex-arb", VenueKind::Dex, "ETH", AccountKind::DexWallet);
        // 新资产首次过账会用请求价格登记一次。
        engine
            .post(req(&eth, EntryKind::Deposit, "ETH", "1", "3000"))
            .unwrap();
        assert_eq!(engine.price("ETH"), dec("3000"));
        assert_eq!(engine.total_equity(), dec("8000"));

        engine.set_price("ETH", dec("2800"));
        assert_eq!(engine.price("ETH"), dec("2800"));
        assert_eq!(engine.total_equity(), dec("7800"));
        let holding = engine
            .balances()
            .into_iter()
            .find(|b| b.account_id == eth)
            .unwrap();
        assert_eq!(holding.balance, dec("1"));
        assert_eq!(holding.usd_value, dec("2800"));
    }

    #[test]
    fn reconcile_flags_mismatch() {
        let engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let cex = account_of(&engine, AccountKind::CexSpot);
        let recs = engine.reconcile(&[
            (dex.clone(), dec("2500")),
            (cex.clone(), dec("2400")),
            ("ghost".to_string(), dec("10")),
        ]);
        assert_eq!(recs.len(), 3);
        assert!(recs[0].ok);
        assert_eq!(recs[0].diff, Decimal::ZERO);
        assert!(!recs[1].ok);
        assert_eq!(recs[1].diff, dec("100"));
        assert!(!recs[2].ok);
        assert_eq!(recs[2].diff, dec("-10"));
        assert!(recs[2].notes.contains("LEDGER_MISMATCH"));
    }

    #[test]
    fn strategy_attribution_tracks_realized_and_costs() {
        let mut engine = engine();
        let rp = account_of(&engine, AccountKind::RealizedPnl);
        let gas = account_of(&engine, AccountKind::GasExpense);
        let mut pnl_ok = req(&rp, EntryKind::Pnl, "USDC", "12", "1");
        pnl_ok.strategy_type = Some(StrategyType::DexTriple);
        pnl_ok.execution_id = Some("exec_1".to_string());
        engine.post(pnl_ok).unwrap();
        let mut pnl_bad = req(&rp, EntryKind::Pnl, "USDC", "-4", "1");
        pnl_bad.strategy_type = Some(StrategyType::CexDex);
        engine.post(pnl_bad).unwrap();
        let mut gas_req = req(&gas, EntryKind::Gas, "USDC", "2", "1");
        gas_req.strategy_type = Some(StrategyType::DexTriple);
        engine.post(gas_req).unwrap();

        assert_eq!(engine.realized_pnl(), dec("8"));
        assert_eq!(
            engine.strategy_realized_pnl(StrategyType::DexTriple),
            dec("12")
        );
        assert_eq!(
            engine.strategy_realized_pnl(StrategyType::CexDex),
            dec("-4")
        );
        assert_eq!(
            engine.strategy_realized_pnl(StrategyType::DexCross),
            Decimal::ZERO
        );
        assert_eq!(engine.strategy_costs(StrategyType::DexTriple), dec("2"));
        assert_eq!(engine.strategy_costs(StrategyType::CexDex), Decimal::ZERO);
    }

    #[test]
    fn query_filters_and_limits() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let gas = account_of(&engine, AccountKind::GasExpense);
        engine
            .post(req(&dex, EntryKind::Gas, "USDC", "-1", "1"))
            .unwrap();
        engine
            .post(req(&gas, EntryKind::Gas, "USDC", "1", "1"))
            .unwrap();

        assert_eq!(
            engine.query(&LedgerQuery::default()).len(),
            engine.entries().len()
        );
        let only_dex = engine.query(&LedgerQuery {
            account_id: Some(dex.clone()),
            ..Default::default()
        });
        assert!(!only_dex.is_empty());
        assert!(only_dex.iter().all(|e| e.account_id == dex));

        let only_gas = engine.query(&LedgerQuery {
            kind: Some("GAS".to_string()),
            ..Default::default()
        });
        assert_eq!(only_gas.len(), 2);

        let last_one = engine.query(&LedgerQuery {
            limit: Some(1),
            ..Default::default()
        });
        assert_eq!(last_one.len(), 1);
        assert_eq!(last_one[0].account_id, gas);
    }

    #[test]
    fn export_restore_roundtrip_preserves_balances() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        let gas = account_of(&engine, AccountKind::GasExpense);
        engine
            .post_batch(vec![
                req(&dex, EntryKind::Gas, "USDC", "-2", "1"),
                req(&gas, EntryKind::Gas, "USDC", "2", "1"),
            ])
            .unwrap();
        engine.record_trade(TradeStat {
            execution_id: "exec_1".to_string(),
            strategy_type: StrategyType::DexCross,
            net_pnl: dec("1.5"),
            gross_pnl: dec("2"),
            cost: dec("0.5"),
            filled_ratio: dec("1"),
            reverted: false,
            exposure_ms: 500,
            notional: dec("100"),
            closed_at: now(),
        });

        let snapshot = engine.export();
        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: LedgerSnapshot = serde_json::from_str(&json).unwrap();
        let restored = LedgerEngine::restore(parsed);

        assert_eq!(restored.accounts().len(), engine.accounts().len());
        assert_eq!(restored.entries().len(), engine.entries().len());
        assert_eq!(restored.total_equity(), engine.total_equity());
        assert_eq!(restored.start_equity(), engine.start_equity());
        assert_eq!(restored.trade_stats().len(), 1);
        for balance in engine.balances() {
            assert_eq!(
                restored.balance(&balance.account_id),
                balance.balance,
                "账户 {} 余额不一致",
                balance.account_id
            );
        }
        // 恢复后仍可继续记账，且流水追加在同一条链上。
        let mut restored = restored;
        restored
            .post(req(&dex, EntryKind::Gas, "USDC", "-1", "1"))
            .unwrap();
        assert_eq!(restored.balance(&dex), dec("2497"));
    }

    #[test]
    fn reset_returns_to_opening_state() {
        let mut engine = engine();
        let dex = account_of(&engine, AccountKind::DexWallet);
        engine
            .post(req(&dex, EntryKind::Trade, "USDC", "-100", "1"))
            .unwrap();
        engine.reset(dec("8000"));
        assert_eq!(engine.total_equity(), dec("8000"));
        assert_eq!(engine.balance(&dex), dec("4000"));
        assert_eq!(engine.entries().len(), 2);
        assert!(engine.trade_stats().is_empty());
    }
}
