//! 启动时的状态恢复与纸面库存初始化（TDD §15 重启恢复）。

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use std::sync::Arc;

use crate::domain::types::{AccountKind, EntryKind, VenueKind};
use crate::engine::AppState;
use crate::ledger::{LedgerEngine, PostRequest};

/// 从数据库恢复最近状态。坏数据只告警，不阻塞启动。
pub async fn restore(state: &Arc<AppState>) -> anyhow::Result<()> {
    let opps = crate::db::repo::load_opportunities(&state.db, 200).unwrap_or_default();
    {
        let mut list = state.opportunities.write().await;
        *list = opps;
    }
    let execs = crate::db::repo::load_executions(&state.db, 200).unwrap_or_default();
    {
        let mut list = state.executions.write().await;
        *list = execs;
    }
    let events = crate::db::repo::load_risk_events(&state.db, 500).unwrap_or_default();
    {
        let mut list = state.risk_events.write().await;
        *list = events;
    }
    if let Ok(Some(controls)) = crate::db::repo::load_controls(&state.db) {
        let mut c = state.controls.write().await;
        *c = controls;
    }
    if let Ok(Some(status)) = crate::db::repo::load_scanner_state(&state.db) {
        let mut s = state.scanner.write().await;
        // 重启后不自动继承 RUNNING：必须由操作者显式启动，避免带着残缺行情继续扫描
        *s = status;
        s.state = "STOPPED".into();
        s.started_at = None;
        s.uptime_ms = 0;
    }
    // 恢复参数覆盖
    if let Ok(overrides) = crate::db::repo::load_overrides(&state.db) {
        let mut risk = state.risk.write().await;
        for ov in overrides {
            let _ = risk.set_override(ov);
        }
    }
    // 账本快照：恢复账户、余额、流水与成交统计（TDD §15 重启恢复）
    if let Ok(Some(raw)) = crate::db::repo::get_config(&state.db, "ledger_snapshot") {
        match serde_json::from_str::<crate::ledger::engine::LedgerSnapshot>(&raw) {
            Ok(snap) => {
                let equity = {
                    let restored = LedgerEngine::restore(snap);
                    let equity = restored.total_equity();
                    *state.ledger.lock().await = restored;
                    equity
                };
                tracing::info!("账本快照已恢复：权益 {equity}");
            }
            Err(e) => tracing::warn!("账本快照解析失败，使用期初账户: {e}"),
        }
    }
    // 价格表
    sync_prices(state).await;
    Ok(())
}

/// 把当前行情价格同步到账本（权益按最新价格计算）。
pub async fn sync_prices(state: &Arc<AppState>) {
    let prices = {
        let m = state.market.read().await;
        m.prices.clone()
    };
    let mut ledger = state.ledger.lock().await;
    for (asset, price) in prices {
        ledger.set_price(&asset, price);
    }
}

/// 初始化纸面库存：让 DEX 与 CEX 两边都持有可交易的基础/计价资产。
///
/// 这一步是「期初资金分配」，不是套利利润；每笔划转都有独立流水与来源说明。
pub async fn seed_inventory(state: &Arc<AppState>) -> anyhow::Result<()> {
    let (weth_price, eth_price, wbtc_price, btc_price) = {
        let m = state.market.read().await;
        (
            m.price("WETH"),
            m.price("ETH"),
            m.price("WBTC"),
            m.price("BTC"),
        )
    };
    if weth_price.is_zero() {
        return Ok(());
    }

    let mut ledger = state.ledger.lock().await;
    // 已经播种过（DEX 钱包已有 WETH）则跳过
    if let Some(acct) = ledger.find_account("dex-arb", "WETH", AccountKind::DexWallet) {
        if ledger.balance(&acct.account_id) > Decimal::ZERO {
            return Ok(());
        }
    }

    let dex_usdc = Decimal::from(400);
    let cex_usdt = Decimal::from(400);
    let eth_qty = Decimal::new(15, 2); // 0.15 ETH
    let eth_cost = eth_qty * weth_price;
    let _ = eth_price;

    let dex_wallet_usdc = ledger.ensure_account("dex-arb", VenueKind::Dex, "USDC", AccountKind::DexWallet);
    let dex_wallet_usdt = ledger.ensure_account("dex-arb", VenueKind::Dex, "USDT", AccountKind::DexWallet);
    let dex_wallet_weth = ledger.ensure_account("dex-arb", VenueKind::Dex, "WETH", AccountKind::DexWallet);
    let cex_spot_usdt = ledger.ensure_account("cex-binance", VenueKind::Cex, "USDT", AccountKind::CexSpot);
    let cex_spot_usdc = ledger.ensure_account("cex-binance", VenueKind::Cex, "USDC", AccountKind::CexSpot);
    let cex_spot_eth = ledger.ensure_account("cex-binance", VenueKind::Cex, "ETH", AccountKind::CexSpot);
    let dex_wallet_wbtc = ledger.ensure_account("dex-arb", VenueKind::Dex, "WBTC", AccountKind::DexWallet);
    let cex_spot_btc = ledger.ensure_account("cex-binance", VenueKind::Cex, "BTC", AccountKind::CexSpot);
    let usdc_id = dex_wallet_usdc.clone();
    let cex_usdt_id = cex_spot_usdt.clone();

    let entry = |account: String, asset: &str, amount: Decimal, price: Decimal, memo: &str| PostRequest {
        account_id: account,
        kind: EntryKind::Transfer,
        asset: asset.to_string(),
        amount,
        price_usd: price,
        strategy_type: None,
        opportunity_id: None,
        plan_id: None,
        execution_id: None,
        ref_id: Some("seed-inventory".into()),
        memo: memo.to_string(),
    };

    let mut batch = vec![
        // DEX：USDC → USDT（稳定币等值划转，保持总权益不变）
        entry(dex_wallet_usdc.clone(), "USDC", -dex_usdc, Decimal::ONE, "期初：DEX 钱包配置 USDT 库存"),
        entry(dex_wallet_usdt, "USDT", dex_usdc, Decimal::ONE, "期初：DEX 钱包收到 USDT"),
        // DEX：USDC → WETH
        entry(dex_wallet_usdc, "USDC", -eth_cost, Decimal::ONE, "期初：DEX 钱包买入 WETH"),
        entry(dex_wallet_weth, "WETH", eth_qty, weth_price, "期初：DEX 钱包持有 WETH"),
        // CEX：USDT → USDC
        entry(cex_spot_usdt.clone(), "USDT", -cex_usdt, Decimal::ONE, "期初：CEX 现货配置 USDC 库存"),
        entry(cex_spot_usdc, "USDC", cex_usdt, Decimal::ONE, "期初：CEX 现货收到 USDC"),
        // CEX：USDT → ETH
        entry(cex_spot_usdt, "USDT", -eth_cost, Decimal::ONE, "期初：CEX 现货买入 ETH"),
        entry(cex_spot_eth, "ETH", eth_qty, eth_price.max(weth_price), "期初：CEX 现货持有 ETH"),
    ];
    // 期初同时配置 BTC 两侧库存，使 CEX–DEX 双向都有可执行样本（Paper）。
    let btc_qty = Decimal::new(2, 2); // 0.02 BTC
    if wbtc_price > Decimal::ZERO {
        batch.push(entry(
            usdc_id.clone(),
            "USDC",
            -(btc_qty * wbtc_price),
            Decimal::ONE,
            "期初：DEX 钱包买入 WBTC",
        ));
        batch.push(entry(
            dex_wallet_wbtc,
            "WBTC",
            btc_qty,
            wbtc_price,
            "期初：DEX 钱包持有 WBTC",
        ));
    }
    if btc_price > Decimal::ZERO {
        batch.push(entry(
            cex_usdt_id.clone(),
            "USDT",
            -(btc_qty * btc_price),
            Decimal::ONE,
            "期初：CEX 现货买入 BTC",
        ));
        batch.push(entry(
            cex_spot_btc,
            "BTC",
            btc_qty,
            btc_price,
            "期初：CEX 现货持有 BTC",
        ));
    }

    match ledger.post_batch(batch) {
        Ok(entries) => {
            let _ = crate::db::repo::append_ledger_entries(&state.db, &entries);
            tracing::info!("期初库存已注入：{} 条流水", entries.len());
        }
        Err(e) => tracing::warn!("期初库存注入失败: {e}"),
    }
    let _ = Utc::now();
    Ok(())
}

/// 启动日志中的资金分配说明。
pub fn allocation_note(start: f64, dex_share: f64) -> String {
    let dex = start * dex_share;
    let cex = start - dex;
    format!(
        "期初资金 {start:.0} USDC：DEX {dex:.0}（{:.0}%）/ CEX {cex:.0}（{:.0}%）",
        dex_share * 100.0,
        (1.0 - dex_share) * 100.0
    )
}

#[allow(dead_code)]
fn dec(v: f64) -> Decimal {
    Decimal::from_f64(v).unwrap_or(Decimal::ZERO)
}
