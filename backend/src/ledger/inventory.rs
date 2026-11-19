//! 场所账户、库存快照与跨场所敞口（TDD §9.3、PRD §5.7）。
//!
//! 口径：
//! - 只有**资产类账户**（DEX 钱包 / CEX 现货 / 闪电贷清算）构成持仓；成本与盈亏账户是统计
//!   账户，不产生库存。
//! - 场所内同一资产的多条账户会合并为一条持仓（例如同一场所的稳定币余额）。
//! - 合并净持仓按「资产」维度跨场所合并，同时保留 `cex_qty` / `dex_qty` 两个场所视角；
//!   `cross_venue_imbalance = cex_qty − dex_qty`，用于判断库存再平衡需求。
//! - Paper 阶段不冻结资金：`available = 余额`，`frozen` / `pending` 恒为 0。
//! - 金额一律用 DECIMAL 精确计算，不使用浮点。

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::domain::ledger::{AssetHolding, CombinedPosition, InventorySnapshot, VenueAccount};
use crate::domain::types::{AccountKind, VenueKind};
use crate::ledger::engine::{LedgerEngine, is_asset_kind};

/// 场所展示名。
fn venue_name(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::DexWallet => "DEX 钱包（Paper）",
        AccountKind::CexSpot => "CEX 现货（Paper）",
        AccountKind::FlashClearing => "闪电贷清算",
        _ => "资金账户",
    }
}

/// 链名展示（第一阶段只有链 ID 1 有名字，其余回退为 `chain:<id>`）。
fn chain_label(engine: &LedgerEngine, venue: &str) -> Option<String> {
    engine
        .accounts()
        .iter()
        .find(|a| a.venue == venue)
        .and_then(|a| a.chain_id)
        .map(|id| {
            if id == 1 {
                "ethereum".to_string()
            } else {
                format!("chain:{id}")
            }
        })
}

/// 最新流水时间（无流水时取当前时间），作为库存快照时间戳。
fn latest_ts(engine: &LedgerEngine) -> DateTime<Utc> {
    engine
        .entries()
        .last()
        .map(|e| e.ts)
        .unwrap_or_else(Utc::now)
}

/// 库存快照：按「场所 + 资产」合并可用/冻结/待结算。
pub fn inventory_snapshots(engine: &LedgerEngine) -> Vec<InventorySnapshot> {
    let ts = latest_ts(engine);
    // BTreeMap 保证输出按 (venue, asset) 稳定排序。
    let mut merged: BTreeMap<(String, String), (VenueKind, Decimal)> = BTreeMap::new();
    for account in engine.accounts() {
        if !is_asset_kind(account.kind) {
            continue;
        }
        let entry = merged
            .entry((account.venue.clone(), account.asset.clone()))
            .or_insert((account.venue_kind, Decimal::ZERO));
        entry.1 += engine.balance(&account.account_id);
    }
    merged
        .into_iter()
        .map(|((venue, asset), (venue_kind, available))| {
            let usd_price = engine.price(&asset);
            InventorySnapshot {
                ts,
                venue,
                venue_kind,
                asset,
                available,
                frozen: Decimal::ZERO,
                pending: Decimal::ZERO,
                usd_price,
                usd_value: available * usd_price,
            }
        })
        .collect()
}

/// 场所账户汇总（账户与库存页面）：按场所聚合资产持仓与 USD 估值。
pub fn venue_accounts(engine: &LedgerEngine) -> Vec<VenueAccount> {
    let mut out: Vec<VenueAccount> = Vec::new();
    for snapshot in inventory_snapshots(engine) {
        let kind = engine
            .accounts()
            .iter()
            .find(|a| a.venue == snapshot.venue)
            .map(|a| a.kind)
            .unwrap_or(AccountKind::DexWallet);
        let holding = AssetHolding {
            asset: snapshot.asset.clone(),
            qty: snapshot.available,
            usd_price: snapshot.usd_price,
            usd_value: snapshot.usd_value,
            available: snapshot.available,
            frozen: snapshot.frozen,
            pending: snapshot.pending,
        };
        match out.iter_mut().find(|v| v.venue == snapshot.venue) {
            Some(venue) => {
                venue.usd_value += holding.usd_value;
                if let Some(existing) = venue.assets.iter_mut().find(|a| a.asset == holding.asset) {
                    existing.qty += holding.qty;
                    existing.usd_value += holding.usd_value;
                    existing.available += holding.available;
                    existing.frozen += holding.frozen;
                    existing.pending += holding.pending;
                } else {
                    venue.assets.push(holding);
                }
            }
            None => out.push(VenueAccount {
                venue: snapshot.venue.clone(),
                name: venue_name(kind).to_string(),
                kind: snapshot.venue_kind,
                chain: chain_label(engine, &snapshot.venue),
                // Paper 阶段场所恒为可用；真实连通性状态由 feed / risk 模块另行上报。
                state: "ONLINE".to_string(),
                usd_value: holding.usd_value,
                assets: vec![holding],
            }),
        }
    }
    out
}

/// 合并净持仓 + 跨场所净敞口。
///
/// `cex_qty` 取 CEX 现货账户余额，`dex_qty` 取 DEX 钱包账户余额，
/// `cross_venue_imbalance = cex_qty − dex_qty`。
pub fn combined_positions(engine: &LedgerEngine) -> Vec<CombinedPosition> {
    let mut cex: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut dex: BTreeMap<String, Decimal> = BTreeMap::new();
    for account in engine.accounts() {
        let target = match account.kind {
            AccountKind::CexSpot => &mut cex,
            AccountKind::DexWallet => &mut dex,
            // 闪电贷清算户是临时本金，不计入跨场所库存敞口。
            _ => continue,
        };
        *target.entry(account.asset.clone()).or_insert(Decimal::ZERO) +=
            engine.balance(&account.account_id);
    }
    let mut assets: Vec<String> = cex.keys().chain(dex.keys()).cloned().collect();
    assets.sort();
    assets.dedup();

    assets
        .into_iter()
        .map(|asset| {
            let cex_qty = cex.get(&asset).copied().unwrap_or(Decimal::ZERO);
            let dex_qty = dex.get(&asset).copied().unwrap_or(Decimal::ZERO);
            let total_qty = cex_qty + dex_qty;
            let usd_price = engine.price(&asset);
            CombinedPosition {
                asset,
                cex_qty,
                dex_qty,
                total_qty,
                usd_price,
                usd_value: total_qty * usd_price,
                cross_venue_imbalance: cex_qty - dex_qty,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dec;
    use crate::domain::types::{AccountKind, EntryKind, VenueKind};
    use crate::ledger::engine::PostRequest;

    fn post(engine: &mut LedgerEngine, account_id: &str, asset: &str, amount: &str, price: &str) {
        engine
            .post(PostRequest::new(
                account_id,
                EntryKind::Deposit,
                asset,
                dec(amount),
                dec(price),
            ))
            .unwrap();
    }

    #[test]
    fn venue_accounts_group_holdings_by_venue() {
        let engine = LedgerEngine::new(dec("5000"));
        let venues = venue_accounts(&engine);
        assert_eq!(venues.len(), 3, "dex-arb / cex-binance / flash 三个场所");

        let dex = venues.iter().find(|v| v.venue == "dex-arb").unwrap();
        assert_eq!(dex.kind, VenueKind::Dex);
        assert_eq!(dex.usd_value, dec("2500"));
        assert_eq!(dex.assets.len(), 1);
        assert_eq!(dex.assets[0].asset, "USDC");
        assert_eq!(dex.assets[0].qty, dec("2500"));
        assert_eq!(dex.assets[0].usd_value, dec("2500"));
        assert_eq!(dex.assets[0].frozen, Decimal::ZERO);
        assert_eq!(dex.chain.as_deref(), Some("ethereum"));

        let cex = venues.iter().find(|v| v.venue == "cex-binance").unwrap();
        assert_eq!(cex.kind, VenueKind::Cex);
        assert_eq!(cex.usd_value, dec("2500"));
        assert_eq!(cex.assets[0].asset, "USDT");
        assert!(cex.chain.is_none());

        // 成本账户（gas / fee）不是场所持仓，不产生额外场所或持仓。
        assert!(venues.iter().all(|v| v.assets.len() == 1));
    }

    #[test]
    fn venue_accounts_merge_same_asset_within_venue() {
        let mut engine = LedgerEngine::new(dec("5000"));
        let eth = engine.ensure_account("dex-arb", VenueKind::Dex, "ETH", AccountKind::DexWallet);
        engine.set_price("ETH", dec("3000"));
        post(&mut engine, &eth, "ETH", "1", "3000");

        let venues = venue_accounts(&engine);
        let dex = venues.iter().find(|v| v.venue == "dex-arb").unwrap();
        assert_eq!(dex.assets.len(), 2);
        let eth = dex.assets.iter().find(|a| a.asset == "ETH").unwrap();
        assert_eq!(eth.qty, dec("1"));
        assert_eq!(eth.usd_value, dec("3000"));
        assert_eq!(dex.usd_value, dec("5500"));
    }

    #[test]
    fn inventory_snapshots_are_sorted_and_only_hold_assets() {
        let engine = LedgerEngine::new(dec("5000"));
        let snaps = inventory_snapshots(&engine);
        assert_eq!(snaps.len(), 3);
        assert!(snaps.iter().all(|s| s.frozen == Decimal::ZERO));
        // 按 (venue, asset) 稳定排序。
        let keys: Vec<(String, String)> = snaps
            .iter()
            .map(|s| (s.venue.clone(), s.asset.clone()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        let dex_usdc = snaps
            .iter()
            .find(|s| s.venue == "dex-arb" && s.asset == "USDC")
            .unwrap();
        assert_eq!(dex_usdc.available, dec("2500"));
        assert_eq!(dex_usdc.usd_value, dec("2500"));
        assert_eq!(dex_usdc.venue_kind, VenueKind::Dex);
    }

    #[test]
    fn combined_positions_report_cross_venue_imbalance() {
        let mut engine = LedgerEngine::new(dec("5000"));
        let eth_cex =
            engine.ensure_account("cex-binance", VenueKind::Cex, "ETH", AccountKind::CexSpot);
        let eth_dex =
            engine.ensure_account("dex-arb", VenueKind::Dex, "ETH", AccountKind::DexWallet);
        engine.set_price("ETH", dec("3000"));
        post(&mut engine, &eth_cex, "ETH", "2", "3000");
        post(&mut engine, &eth_dex, "ETH", "1.5", "3000");

        let positions = combined_positions(&engine);
        let eth = positions.iter().find(|p| p.asset == "ETH").unwrap();
        assert_eq!(eth.cex_qty, dec("2"));
        assert_eq!(eth.dex_qty, dec("1.5"));
        assert_eq!(eth.total_qty, dec("3.5"));
        assert_eq!(eth.cross_venue_imbalance, dec("0.5"));
        assert_eq!(eth.usd_price, dec("3000"));
        assert_eq!(eth.usd_value, dec("10500"));

        // 初始 USDC / USDT 各自单边持有：敞口等于单边数量。
        let usdc = positions.iter().find(|p| p.asset == "USDC").unwrap();
        assert_eq!(usdc.cex_qty, Decimal::ZERO);
        assert_eq!(usdc.dex_qty, dec("2500"));
        assert_eq!(usdc.cross_venue_imbalance, dec("-2500"));

        let usdt = positions.iter().find(|p| p.asset == "USDT").unwrap();
        assert_eq!(usdt.cex_qty, dec("2500"));
        assert_eq!(usdt.dex_qty, Decimal::ZERO);
        assert_eq!(usdt.cross_venue_imbalance, dec("2500"));

        // 排序稳定（按资产名）。
        let assets: Vec<&str> = positions.iter().map(|p| p.asset.as_str()).collect();
        let mut sorted = assets.clone();
        sorted.sort();
        assert_eq!(assets, sorted);
    }
}
