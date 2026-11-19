//! 绩效原始聚合（PRD §7 Paper 验收、风控 §11 统计要求）。
//!
//! # 净盈亏口径（与 [`crate::ledger::engine`] 的记账口径一致）
//!
//! 账本把「毛已实现盈亏」与「成本」分开记账：
//!
//! - `REALIZED_PNL` 账户 = 已实现**毛**盈亏（策略点差收益，不含任何成本）；
//! - `CEX_FEE` / `DEX_FEE` / `GAS_EXPENSE` / `HEDGE_COMPENSATION` /
//!   `INVENTORY_REBALANCE` 账户 = 各自成本，正数累计；
//! - 资产类账户余额 = 真实资金。
//!
//! 因此本模块统一按
//!
//! ```text
//! net_pnl = realized_pnl + unrealized_pnl
//!         − total_fees − total_gas − total_compensation − total_rebalance
//! ```
//!
//! 计算净盈亏，**每个成本项只扣一次**（`REALIZED_PNL` 里不能再预先扣费）。
//! 若结算时资金流水与 `REALIZED_PNL` 镜像入账（推荐做法），则同时满足
//! `equity − start_equity == net_pnl`（无外部资金注入时）。
//!
//! `TradeStat.net_pnl` 是**逐笔净额**，只用于胜率、分布、回撤、成交率等成交统计，
//! 不参与上面的总额公式，避免重复扣费；`by_strategy` 的 `net_profit` 之和应等于全部
//! `TradeStat.net_pnl` 之和。
//!
//! 说明：夏普比率、资金利用率等比例指标允许用 f64 近似（仅统计展示），
//! 任何资金与盈亏金额都使用 [`rust_decimal::Decimal`] 精确计算。

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::domain::money::pct;
use crate::domain::report::{
    DistributionBin, EquityPoint, Kpi, PerformanceReport, StrategyPerformance,
};
use crate::domain::types::StrategyType;
use crate::ledger::engine::{LedgerEngine, TradeStat};

/// 净利润分布区间（8 档，围绕 USDC 单笔盈亏 -5 ~ +5 展开）。
fn distribution_bins() -> Vec<DistributionBin> {
    let edges: [(f64, f64, &str); 8] = [
        (-1_000_000.0, -5.0, "≤ -5"),
        (-5.0, -2.0, "-5 ~ -2"),
        (-2.0, -1.0, "-2 ~ -1"),
        (-1.0, 0.0, "-1 ~ 0"),
        (0.0, 1.0, "0 ~ 1"),
        (1.0, 2.0, "1 ~ 2"),
        (2.0, 5.0, "2 ~ 5"),
        (5.0, 1_000_000.0, "≥ 5"),
    ];
    edges
        .iter()
        .map(|(from, to, label)| DistributionBin {
            label: (*label).to_string(),
            from: Decimal::try_from(*from).unwrap_or(Decimal::ZERO),
            to: Decimal::try_from(*to).unwrap_or(Decimal::ZERO),
            count: 0,
        })
        .collect()
}

/// 权益序列：按 `TradeStat.closed_at` 的日期累计净盈亏，并计算峰值与回撤。
///
/// - `equity` = `start_equity` + 累计 `net_pnl`；
/// - `pnl` = 累计 `net_pnl`；
/// - `drawdown` = 历史峰值（含期初权益）− 当前权益，正数表示回撤；
/// - `drawdown_pct` = `drawdown / peak × 100`。
///
/// 同一交易日的多笔成交合并为一个点（时间戳取该日 00:00 UTC）。
pub fn equity_series(trades: &[TradeStat], start_equity: Decimal) -> Vec<EquityPoint> {
    let mut by_day: BTreeMap<NaiveDate, Decimal> = BTreeMap::new();
    for trade in trades {
        *by_day
            .entry(trade.closed_at.date_naive())
            .or_insert(Decimal::ZERO) += trade.net_pnl;
    }

    let mut out = Vec::with_capacity(by_day.len());
    let mut cumulative = Decimal::ZERO;
    let mut peak = start_equity;
    for (day, day_pnl) in by_day {
        cumulative += day_pnl;
        let equity = start_equity + cumulative;
        if equity > peak {
            peak = equity;
        }
        let drawdown = (peak - equity).max(Decimal::ZERO);
        out.push(EquityPoint {
            ts: DateTime::<Utc>::from_naive_utc_and_offset(day.and_time(NaiveTime::MIN), Utc),
            equity,
            pnl: cumulative,
            drawdown,
            drawdown_pct: pct(drawdown, peak),
        });
    }
    out
}

/// 由账本 + `TradeStat` 生成完整绩效报告。
///
/// `series` 按成交日聚合；最后一个点对齐 `generated_at` 的日期并用**账本实测权益**校准，
/// 因此报告里始终至少包含一天，且权益曲线终点与 `equity` 一致。
pub fn build_report(
    engine: &LedgerEngine,
    start_equity: Decimal,
    unrealized_pnl: Decimal,
    generated_at: DateTime<Utc>,
) -> PerformanceReport {
    let mut trades: Vec<TradeStat> = engine.trade_stats().to_vec();
    trades.sort_by(|a, b| a.closed_at.cmp(&b.closed_at));

    let equity = engine.total_equity();
    let realized_pnl = engine.realized_pnl();
    let total_fees = engine.total_fees();
    let total_gas = engine.total_gas();
    let total_compensation = engine.total_compensation();
    let total_rebalance = engine.total_rebalance();
    // 唯一净盈亏口径：毛已实现 + 未实现 − 全部成本（每项一次）。
    let net_pnl = realized_pnl + unrealized_pnl
        - total_fees
        - total_gas
        - total_compensation
        - total_rebalance;

    let trade_count = trades.len() as u64;
    let count = Decimal::from(trade_count);
    let wins = trades.iter().filter(|t| t.net_pnl > Decimal::ZERO).count() as u64;
    let losses = trades.iter().filter(|t| t.net_pnl < Decimal::ZERO).count() as u64;

    let gross_profit: Decimal = trades
        .iter()
        .filter(|t| t.net_pnl > Decimal::ZERO)
        .map(|t| t.net_pnl)
        .sum();
    let gross_loss: Decimal = trades
        .iter()
        .filter(|t| t.net_pnl < Decimal::ZERO)
        .map(|t| -t.net_pnl)
        .sum();
    let profit_factor = if gross_loss > Decimal::ZERO {
        gross_profit / gross_loss
    } else if gross_profit > Decimal::ZERO {
        // 无亏损交易时盈亏比无上界，用 999 作为「极大」哨兵值（前端按 ≥999 展示）。
        Decimal::from(999)
    } else {
        Decimal::ZERO
    };

    let notional_sum: Decimal = trades.iter().map(|t| t.notional).sum();
    let fill_sum: Decimal = trades.iter().map(|t| t.filled_ratio).sum();
    let reverted = trades.iter().filter(|t| t.reverted).count() as u64;
    let exposure_sum: i64 = trades.iter().map(|t| t.exposure_ms).sum();
    let avg_hold_ms = if trade_count == 0 {
        0
    } else {
        exposure_sum / trade_count as i64
    };

    // 权益曲线：成交日序列 + 末点用账本实测权益校准。
    let mut series = equity_series(&trades, start_equity);
    let generated_day = generated_at.date_naive();
    match series.last() {
        Some(last) if last.ts.date_naive() == generated_day => {
            let last = series.last_mut().expect("序列非空");
            last.equity = equity;
            last.pnl = equity - start_equity;
        }
        Some(last) if last.ts.date_naive() > generated_day => {
            // 生成时间早于最后一笔成交：保留成交推导出的序列。
        }
        _ => series.push(EquityPoint {
            ts: generated_at,
            equity,
            pnl: equity - start_equity,
            drawdown: Decimal::ZERO,
            drawdown_pct: Decimal::ZERO,
        }),
    }
    recompute_drawdown(&mut series, start_equity);

    let max_drawdown = series
        .iter()
        .map(|p| p.drawdown_pct)
        .max()
        .unwrap_or(Decimal::ZERO);
    let max_drawdown_usd = series
        .iter()
        .map(|p| p.drawdown)
        .max()
        .unwrap_or(Decimal::ZERO);

    let mut report = PerformanceReport::empty(equity);
    report.generated_at = generated_at;
    report.start_equity = start_equity;
    report.equity = equity;
    report.realized_pnl = realized_pnl;
    report.unrealized_pnl = unrealized_pnl;
    report.total_fees = total_fees;
    report.total_gas = total_gas;
    report.total_compensation = total_compensation;
    report.total_rebalance = total_rebalance;
    report.net_pnl = net_pnl;
    report.total_return_pct = pct(net_pnl, start_equity);
    report.max_drawdown = max_drawdown;
    report.max_drawdown_usd = max_drawdown_usd;
    report.sharpe = sharpe(&series);
    report.profit_factor = profit_factor;
    report.trades = trade_count;
    report.wins = wins;
    report.losses = losses;
    report.win_rate = pct(Decimal::from(wins), count);
    report.avg_hold_ms = avg_hold_ms;
    report.fill_rate = pct(fill_sum, count);
    report.revert_rate = pct(Decimal::from(reverted), count);
    report.capital_utilization = pct(notional_sum, start_equity);
    report.series = series;
    // 三种策略恒出现（无交易时数值为 0），保证前端表格结构稳定。
    report.by_strategy = StrategyType::ALL
        .iter()
        .map(|strategy| strategy_performance(*strategy, &trades, start_equity))
        .collect();
    report.distribution = distribution(&trades);
    let kpis = kpis(&report);
    report.kpis = kpis;
    report
}

/// 单策略绩效。
fn strategy_performance(
    strategy: StrategyType,
    trades: &[TradeStat],
    start_equity: Decimal,
) -> StrategyPerformance {
    let own: Vec<&TradeStat> = trades
        .iter()
        .filter(|t| t.strategy_type == strategy)
        .collect();
    let trade_count = own.len();
    let count = Decimal::from(trade_count as u64);
    let wins = own.iter().filter(|t| t.net_pnl > Decimal::ZERO).count() as u64;
    let losses = own.iter().filter(|t| t.net_pnl < Decimal::ZERO).count() as u64;

    let gross_profit: Decimal = own.iter().map(|t| t.gross_pnl).sum();
    let total_cost: Decimal = own.iter().map(|t| t.cost).sum();
    let net_profit: Decimal = own.iter().map(|t| t.net_pnl).sum();
    let notional: Decimal = own.iter().map(|t| t.notional).sum();
    let fill_sum: Decimal = own.iter().map(|t| t.filled_ratio).sum();
    let reverted = own.iter().filter(|t| t.reverted).count() as u64;
    let exposure_sum: i64 = own.iter().map(|t| t.exposure_ms).sum();

    StrategyPerformance {
        strategy_type: strategy,
        label: strategy.label().to_string(),
        trades: trade_count as u64,
        wins,
        losses,
        win_rate: pct(Decimal::from(wins), count),
        // 毛盈亏合计（未扣成本）− 成本合计 = 净盈亏，三者可逐笔核对。
        gross_profit,
        total_cost,
        net_profit,
        avg_net_profit: if trade_count == 0 {
            Decimal::ZERO
        } else {
            net_profit / count
        },
        max_drawdown: max_drawdown_amount(&own),
        fill_rate: pct(fill_sum, count),
        revert_rate: pct(Decimal::from(reverted), count),
        avg_exposure_ms: if trade_count == 0 {
            0
        } else {
            exposure_sum / trade_count as i64
        },
        capital_utilization: pct(notional, start_equity),
    }
}

/// 某策略累计净盈亏的最大回撤金额（USDC，正数）。
fn max_drawdown_amount(trades: &[&TradeStat]) -> Decimal {
    let mut cumulative = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut max_drawdown = Decimal::ZERO;
    for trade in trades {
        cumulative += trade.net_pnl;
        if cumulative > peak {
            peak = cumulative;
        }
        let drawdown = peak - cumulative;
        if drawdown > max_drawdown {
            max_drawdown = drawdown;
        }
    }
    max_drawdown
}

/// 重新计算整条序列的峰值与回撤（峰值起点为期初权益）。
fn recompute_drawdown(series: &mut [EquityPoint], start_equity: Decimal) {
    let mut peak = start_equity;
    for point in series.iter_mut() {
        if point.equity > peak {
            peak = point.equity;
        }
        let drawdown = (peak - point.equity).max(Decimal::ZERO);
        point.drawdown = drawdown;
        point.drawdown_pct = pct(drawdown, peak);
    }
}

/// 净盈亏分布：8 个区间，按 `[from, to)` 计数（最后一档包含上界）。
fn distribution(trades: &[TradeStat]) -> Vec<DistributionBin> {
    let mut bins = distribution_bins();
    for trade in trades {
        let value = trade.net_pnl;
        let index = bins
            .iter()
            .position(|bin| value < bin.to)
            .unwrap_or(bins.len() - 1);
        bins[index].count += 1;
    }
    bins
}

/// 夏普比率（按日权益收益率，年化系数 √365）。仅统计展示，使用 f64 近似。
fn sharpe(series: &[EquityPoint]) -> Decimal {
    if series.len() < 3 {
        return Decimal::ZERO;
    }
    let mut returns: Vec<f64> = Vec::new();
    for pair in series.windows(2) {
        let prev = pair[0].equity.to_f64().unwrap_or(0.0);
        let next = pair[1].equity.to_f64().unwrap_or(0.0);
        if prev > 0.0 {
            returns.push((next - prev) / prev);
        }
    }
    if returns.len() < 2 {
        return Decimal::ZERO;
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let variance = returns.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / n;
    let std_dev = variance.sqrt();
    if !(std_dev > 0.0) || !std_dev.is_finite() {
        return Decimal::ZERO;
    }
    let value = mean / std_dev * 365.0_f64.sqrt();
    if !value.is_finite() {
        return Decimal::ZERO;
    }
    Decimal::try_from(value)
        .map(|d| d.round_dp(4))
        .unwrap_or(Decimal::ZERO)
}

/// 关键指标列表（带单位与口径说明）。
fn kpis(report: &PerformanceReport) -> Vec<Kpi> {
    let item = |key: &str, label: &str, value: Decimal, unit: &str, note: &str| Kpi {
        key: key.to_string(),
        label: label.to_string(),
        value,
        unit: unit.to_string(),
        note: note.to_string(),
        delta: None,
    };
    vec![
        item(
            "equity",
            "总权益",
            report.equity,
            "USDC",
            "资产类账户余额 × 价格之和（不含成本账户）",
        ),
        item(
            "net_pnl",
            "净盈亏",
            report.net_pnl,
            "USDC",
            "已实现毛盈亏 + 未实现盈亏 − 费用 − gas − 补偿 − 再平衡",
        ),
        item(
            "total_return_pct",
            "总收益率",
            report.total_return_pct,
            "%",
            "净盈亏 / 期初权益",
        ),
        item(
            "realized_pnl",
            "已实现盈亏",
            report.realized_pnl,
            "USDC",
            "REALIZED_PNL 账户净额（口径为毛盈亏，成本单列）",
        ),
        item(
            "unrealized_pnl",
            "未实现盈亏",
            report.unrealized_pnl,
            "USDC",
            "按标记价重估，未计入权益",
        ),
        item(
            "total_fees",
            "交易手续费",
            report.total_fees,
            "USDC",
            "CEX_FEE + DEX_FEE",
        ),
        item(
            "total_gas",
            "Gas 成本",
            report.total_gas,
            "USDC",
            "GAS_EXPENSE 累计（含回滚交易的 Gas 损失）",
        ),
        item(
            "total_compensation",
            "对冲补偿",
            report.total_compensation,
            "USDC",
            "HEDGE_COMPENSATION 累计",
        ),
        item(
            "total_rebalance",
            "库存再平衡成本",
            report.total_rebalance,
            "USDC",
            "INVENTORY_REBALANCE 累计（不隐藏进策略利润）",
        ),
        item(
            "max_drawdown",
            "最大回撤",
            report.max_drawdown,
            "%",
            "相对历史峰值（含期初权益）",
        ),
        item(
            "win_rate",
            "胜率",
            report.win_rate,
            "%",
            "净盈亏 > 0 的成交占比",
        ),
        item(
            "profit_factor",
            "盈亏比",
            report.profit_factor,
            "x",
            "盈利成交合计 / 亏损成交合计（无亏损时为 999）",
        ),
        item(
            "trades",
            "成交笔数",
            Decimal::from(report.trades),
            "笔",
            "已结算交易统计条数",
        ),
        item(
            "fill_rate",
            "成交率",
            report.fill_rate,
            "%",
            "CEX 实际成交 / 请求数量",
        ),
        item(
            "revert_rate",
            "回滚率",
            report.revert_rate,
            "%",
            "DEX 回滚成交占比",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dec;
    use crate::domain::types::{AccountKind, EntryKind};
    use crate::ledger::engine::PostRequest;

    fn day(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        let date = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        DateTime::<Utc>::from_naive_utc_and_offset(date.and_hms_opt(12, 0, 0).unwrap(), Utc)
    }

    fn trade(
        execution_id: &str,
        strategy: StrategyType,
        net_pnl: &str,
        gross_pnl: &str,
        cost: &str,
        filled_ratio: &str,
        reverted: bool,
        exposure_ms: i64,
        notional: &str,
        closed_at: DateTime<Utc>,
    ) -> TradeStat {
        TradeStat {
            execution_id: execution_id.to_string(),
            strategy_type: strategy,
            net_pnl: dec(net_pnl),
            gross_pnl: dec(gross_pnl),
            cost: dec(cost),
            filled_ratio: dec(filled_ratio),
            reverted,
            exposure_ms,
            notional: dec(notional),
            closed_at,
        }
    }

    fn account_of(engine: &LedgerEngine, kind: AccountKind) -> String {
        engine
            .accounts()
            .iter()
            .find(|a| a.kind == kind)
            .unwrap()
            .account_id
            .clone()
    }

    #[test]
    fn equity_series_tracks_peak_and_drawdown() {
        let trades = vec![
            trade(
                "e1",
                StrategyType::DexTriple,
                "20",
                "21",
                "1",
                "1",
                false,
                100,
                "1000",
                day(2026, 1, 1),
            ),
            trade(
                "e2",
                StrategyType::DexTriple,
                "-50",
                "-49",
                "1",
                "1",
                false,
                100,
                "1000",
                day(2026, 1, 2),
            ),
            trade(
                "e3",
                StrategyType::CexDex,
                "10",
                "11",
                "1",
                "1",
                false,
                100,
                "1000",
                day(2026, 1, 3),
            ),
        ];
        let series = equity_series(&trades, dec("5000"));
        assert_eq!(series.len(), 3);

        assert_eq!(series[0].equity, dec("5020"));
        assert_eq!(series[0].pnl, dec("20"));
        assert_eq!(series[0].drawdown, Decimal::ZERO);
        assert_eq!(series[0].drawdown_pct, Decimal::ZERO);

        assert_eq!(series[1].equity, dec("4970"));
        assert_eq!(series[1].pnl, dec("-30"));
        assert_eq!(series[1].drawdown, dec("50"), "峰值 5020 − 4970");
        // 50 / 5020 * 100
        assert!(
            (series[1].drawdown_pct - dec("0.9960159362549800796812749004")).abs()
                < dec("0.000001")
        );

        // 反弹到 4980，回撤收敛到 40 而不是归零。
        assert_eq!(series[2].equity, dec("4980"));
        assert_eq!(series[2].drawdown, dec("40"));
    }

    #[test]
    fn equity_series_aggregates_same_day_trades() {
        let trades = vec![
            trade(
                "e1",
                StrategyType::DexCross,
                "10",
                "10",
                "0",
                "1",
                false,
                10,
                "100",
                day(2026, 2, 1),
            ),
            trade(
                "e2",
                StrategyType::DexCross,
                "-4",
                "-4",
                "0",
                "1",
                true,
                10,
                "100",
                day(2026, 2, 1),
            ),
        ];
        let series = equity_series(&trades, dec("1000"));
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].equity, dec("1006"));
        assert_eq!(series[0].pnl, dec("6"));
        assert_eq!(series[0].drawdown, Decimal::ZERO);
        assert!(equity_series(&[], dec("1000")).is_empty());
    }

    #[test]
    fn build_report_totals_match_components_and_inputs() {
        let mut engine = LedgerEngine::new(dec("5000"));
        let dex = account_of(&engine, AccountKind::DexWallet);
        let cex = account_of(&engine, AccountKind::CexSpot);
        let gas = account_of(&engine, AccountKind::GasExpense);
        let cex_fee = account_of(&engine, AccountKind::CexFee);
        let rp = account_of(&engine, AccountKind::RealizedPnl);

        // 第 1 笔：毛 +30，成本 1.5（手续费 1 + gas 0.5）→ 净 28.5
        engine
            .post_batch(vec![
                PostRequest::new(&dex, EntryKind::Trade, "USDC", dec("30"), Decimal::ONE),
                PostRequest::new(&rp, EntryKind::Pnl, "USDC", dec("30"), Decimal::ONE),
                PostRequest::new(&dex, EntryKind::Gas, "USDC", dec("-0.5"), Decimal::ONE),
                PostRequest::new(&gas, EntryKind::Gas, "USDC", dec("0.5"), Decimal::ONE),
                PostRequest::new(&cex, EntryKind::Fee, "USDT", dec("-1"), Decimal::ONE),
                PostRequest::new(&cex_fee, EntryKind::Fee, "USDT", dec("1"), Decimal::ONE),
            ])
            .unwrap();
        // 第 2 笔：毛 -9，成本 1（手续费）→ 净 -10
        engine
            .post_batch(vec![
                PostRequest::new(&cex, EntryKind::Trade, "USDT", dec("-9"), Decimal::ONE),
                PostRequest::new(&rp, EntryKind::Pnl, "USDC", dec("-9"), Decimal::ONE),
                PostRequest::new(&cex, EntryKind::Fee, "USDT", dec("-1"), Decimal::ONE),
                PostRequest::new(&cex_fee, EntryKind::Fee, "USDT", dec("1"), Decimal::ONE),
            ])
            .unwrap();

        engine.record_trade(trade(
            "exec_1",
            StrategyType::DexTriple,
            "28.5",
            "30",
            "1.5",
            "1",
            false,
            800,
            "1000",
            day(2026, 1, 5),
        ));
        engine.record_trade(trade(
            "exec_2",
            StrategyType::CexDex,
            "-10",
            "-9",
            "1",
            "0.5",
            true,
            1200,
            "500",
            day(2026, 1, 6),
        ));

        let generated_at = day(2026, 1, 7);
        let report = build_report(&engine, dec("5000"), Decimal::ZERO, generated_at);

        // 账本侧
        assert_eq!(report.equity, dec("5018.5"));
        assert_eq!(report.realized_pnl, dec("21"));
        assert_eq!(report.total_fees, dec("2"));
        assert_eq!(report.total_gas, dec("0.5"));
        assert_eq!(report.total_compensation, Decimal::ZERO);
        assert_eq!(report.total_rebalance, Decimal::ZERO);
        assert_eq!(report.net_pnl, dec("18.5"));
        // 口径自洽：合计 = 分项求和；资产流水镜像入账时 equity − start == net_pnl。
        assert_eq!(
            report.net_pnl,
            report.realized_pnl + report.unrealized_pnl
                - report.total_fees
                - report.total_gas
                - report.total_compensation
                - report.total_rebalance
        );
        assert_eq!(report.equity - report.start_equity, report.net_pnl);
        assert_eq!(report.total_return_pct, dec("0.37"));

        // 成交统计与输入一致
        assert_eq!(report.trades, 2);
        assert_eq!(report.wins, 1);
        assert_eq!(report.losses, 1);
        assert_eq!(report.win_rate, dec("50"));
        // 盈亏比 = 28.5 / 10
        assert_eq!(report.profit_factor, dec("2.85"));
        assert_eq!(report.avg_hold_ms, 1000);
        assert_eq!(report.fill_rate, dec("75"));
        assert_eq!(report.revert_rate, dec("50"));
        // 资金利用率 = (1000 + 500) / 5000 * 100
        assert_eq!(report.capital_utilization, dec("30"));

        // 权益曲线：两个成交日 + 生成日（末点用账本权益校准）
        assert_eq!(report.series.len(), 3);
        assert_eq!(report.series[0].equity, dec("5028.5"));
        assert_eq!(report.series[1].equity, dec("5018.5"));
        assert_eq!(report.series[2].equity, dec("5018.5"));
        assert_eq!(report.series[2].ts, generated_at);
        // 最大回撤金额 = 峰值 5028.5 − 谷值 5018.5
        assert_eq!(report.max_drawdown_usd, dec("10"));

        // 三种策略恒出现，且明细求和等于合计。
        assert_eq!(report.by_strategy.len(), 3);
        let detail_net: Decimal = report.by_strategy.iter().map(|s| s.net_profit).sum();
        assert_eq!(detail_net, report.net_pnl);
        let detail_gross: Decimal = report.by_strategy.iter().map(|s| s.gross_profit).sum();
        let detail_cost: Decimal = report.by_strategy.iter().map(|s| s.total_cost).sum();
        assert_eq!(detail_gross - detail_cost, detail_net);
        let dex_triple = report
            .by_strategy
            .iter()
            .find(|s| s.strategy_type == StrategyType::DexTriple)
            .unwrap();
        assert_eq!(dex_triple.trades, 1);
        assert_eq!(dex_triple.wins, 1);
        assert_eq!(dex_triple.net_profit, dec("28.5"));
        let dex_cross = report
            .by_strategy
            .iter()
            .find(|s| s.strategy_type == StrategyType::DexCross)
            .unwrap();
        assert_eq!(dex_cross.trades, 0);
        assert_eq!(dex_cross.net_profit, Decimal::ZERO);

        // 分布：8 档，且计数总和等于成交数。
        assert_eq!(report.distribution.len(), 8);
        assert_eq!(report.distribution.iter().map(|b| b.count).sum::<u64>(), 2);
        let top = report.distribution.last().unwrap();
        assert_eq!(top.count, 1, "28.5 落入 ≥5 档");
        let bottom = report.distribution.first().unwrap();
        assert_eq!(bottom.count, 1, "-10 落入 ≤-5 档");

        assert!(!report.kpis.is_empty());
        assert!(report.kpis.iter().any(|k| k.key == "net_pnl"));
    }

    #[test]
    fn build_report_without_trades_has_full_structure() {
        let engine = LedgerEngine::new(dec("5000"));
        let report = build_report(&engine, dec("5000"), dec("12.5"), day(2026, 3, 1));
        assert_eq!(report.equity, dec("5000"));
        assert_eq!(report.trades, 0);
        assert_eq!(report.wins, 0);
        assert_eq!(report.losses, 0);
        assert_eq!(report.win_rate, Decimal::ZERO);
        assert_eq!(report.profit_factor, Decimal::ZERO);
        assert_eq!(report.net_pnl, dec("12.5"));
        assert_eq!(report.by_strategy.len(), 3);
        assert!(
            report
                .by_strategy
                .iter()
                .all(|s| s.trades == 0 && s.net_profit == Decimal::ZERO)
        );
        assert_eq!(report.distribution.len(), 8);
        assert!(report.distribution.iter().all(|b| b.count == 0));
        // 无成交时仍给出最后一天的点。
        assert_eq!(report.series.len(), 1);
        assert_eq!(report.series[0].equity, dec("5000"));
        assert_eq!(report.max_drawdown, Decimal::ZERO);
        assert!(!report.kpis.is_empty());
    }

    #[test]
    fn build_report_marks_unrealized_and_ledger_costs() {
        let mut engine = LedgerEngine::new(dec("5000"));
        let dex = account_of(&engine, AccountKind::DexWallet);
        let gas = account_of(&engine, AccountKind::GasExpense);
        // 成对结算：资产账户流出 -0.5，gas 成本账户 +0.5。
        engine
            .post_batch(vec![
                PostRequest::new(&dex, EntryKind::Gas, "USDC", dec("-0.5"), Decimal::ONE),
                PostRequest::new(&gas, EntryKind::Gas, "USDC", dec("0.5"), Decimal::ONE),
            ])
            .unwrap();
        let report = build_report(&engine, dec("5000"), dec("3"), day(2026, 4, 1));
        assert_eq!(report.total_gas, dec("0.5"));
        assert_eq!(report.net_pnl, dec("2.5"));
        assert_eq!(report.equity, dec("4999.5"));
    }
}
