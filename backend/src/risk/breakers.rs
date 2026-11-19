//! 熔断器：滑动窗口计数与资金类熔断（风控 §10）。
//!
//! 所有窗口按时间/次数裁剪，避免旧记录长期占用熔断额度：
//! - DEX 回滚：15 分钟窗口，3 次触发链级熔断；
//! - 对冲异常：最近 10 次尝试窗口，2 次触发交易对熔断；
//! - 资金类：日亏损 1%、策略日亏损 0.5%、回撤 5%。

use std::collections::{BTreeMap, VecDeque};

use chrono::{DateTime, Duration, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use crate::domain::types::{RiskEventKind, RiskScope, RiskSeverity};
use crate::risk::catalog;

/// DEX 回滚滑动窗口（分钟）。
pub const DEX_REVERT_WINDOW_MINS: i64 = 15;
/// 窗口内 DEX 回滚次数上限。
pub const DEX_REVERT_LIMIT: usize = 3;
/// 链级熔断建议最短等待时间（秒）。
pub const DEX_REVERT_COOLDOWN_SECS: u64 = 1_800;
/// 对冲异常熔断建议最短等待时间（秒），恢复需完成订单/库存/账本核对。
pub const HEDGE_ANOMALY_COOLDOWN_SECS: u64 = 1_800;

/// 一次熔断触发：包含范围、原因与建议暂停时长。
#[derive(Clone, Debug)]
pub struct BreakerTrigger {
    pub kind: RiskEventKind,
    pub severity: RiskSeverity,
    pub scope: RiskScope,
    pub scope_ref: String,
    pub message: String,
    /// 建议的最短暂停时间（秒）
    pub cooldown_secs: u64,
}

/// 熔断器：只保存滑动窗口状态，参数阈值取自 `catalog::baseline()`。
#[derive(Debug, Default)]
pub struct CircuitBreakers {
    /// 每条链最近 15 分钟的回滚时间点。
    dex_reverts: BTreeMap<u64, VecDeque<DateTime<Utc>>>,
    /// 每个交易对最近 N 次对冲尝试（`true` = 异常）。
    hedge_attempts: BTreeMap<String, VecDeque<bool>>,
}

impl CircuitBreakers {
    pub fn new() -> Self {
        Self::default()
    }

    /// 15 分钟内 3 次 DEX 回滚 → 触发链级熔断。
    pub fn record_dex_revert(
        &mut self,
        chain_id: u64,
        now: DateTime<Utc>,
    ) -> Option<BreakerTrigger> {
        let window = self.dex_reverts.entry(chain_id).or_default();
        window.push_back(now);
        Self::prune_dex(window, now);
        let count = window.len();
        if count >= DEX_REVERT_LIMIT {
            Some(BreakerTrigger {
                kind: RiskEventKind::DexReverts,
                severity: RiskSeverity::HardStop,
                scope: RiskScope::Chain,
                scope_ref: chain_id.to_string(),
                message: format!(
                    "链 {} 在 15 分钟内发生 {} 次 DEX 回滚，触发链级熔断：至少等待 30 分钟并检查回滚原因",
                    chain_id, count
                ),
                cooldown_secs: DEX_REVERT_COOLDOWN_SECS,
            })
        } else {
            None
        }
    }

    /// 记录一次对冲尝试（`anomaly=false` 为正常尝试），窗口最多保留 N 次。
    ///
    /// 保留该入口是为了让正常尝试也能把异常记录挤出「最近 10 次」窗口。
    pub fn record_hedge_attempt(
        &mut self,
        symbol: &str,
        anomaly: bool,
        _now: DateTime<Utc>,
    ) -> Option<BreakerTrigger> {
        let window_max = baseline_usize("hedge_anomaly_window", 10).max(1);
        let limit = baseline_usize("hedge_anomaly_limit", 2).max(1);
        let attempts = self.hedge_attempts.entry(symbol.to_string()).or_default();
        attempts.push_back(anomaly);
        while attempts.len() > window_max {
            attempts.pop_front();
        }
        let anomalies = attempts.iter().filter(|a| **a).count();
        if anomalies >= limit {
            Some(BreakerTrigger {
                kind: RiskEventKind::HedgeAnomaly,
                severity: RiskSeverity::HardStop,
                scope: RiskScope::Symbol,
                scope_ref: symbol.to_string(),
                message: format!(
                    "交易对 {} 最近 {} 次尝试中 {} 次对冲异常，暂停该交易对并完成订单、库存和账本核对",
                    symbol,
                    attempts.len(),
                    anomalies
                ),
                cooldown_secs: HEDGE_ANOMALY_COOLDOWN_SECS,
            })
        } else {
            None
        }
    }

    /// 最近 10 次尝试中 2 次对冲异常 → 暂停交易对。
    pub fn record_hedge_anomaly(
        &mut self,
        symbol: &str,
        now: DateTime<Utc>,
    ) -> Option<BreakerTrigger> {
        self.record_hedge_attempt(symbol, true, now)
    }

    /// 资金类熔断：日亏损 1%、策略日亏损 0.5%、回撤 5%。
    pub fn check_loss(
        &mut self,
        equity_usd: Decimal,
        peak_equity_usd: Decimal,
        account_daily_loss_usd: Decimal,
        strategy_losses: &[(String, Decimal)],
    ) -> Vec<BreakerTrigger> {
        let mut out = Vec::new();

        if equity_usd > Decimal::ZERO {
            let global_pct = baseline_dec("global_daily_loss_pct", Decimal::ONE);
            let global_limit = equity_usd * global_pct / Decimal::from(100);
            if account_daily_loss_usd > global_limit {
                out.push(BreakerTrigger {
                    kind: RiskEventKind::DailyLoss,
                    severity: RiskSeverity::HardStop,
                    scope: RiskScope::Global,
                    scope_ref: "GLOBAL".to_string(),
                    message: format!(
                        "全局日亏损 {} USDC 超过净值 {} USDC 的 {}%（上限 {} USDC），全局暂停",
                        account_daily_loss_usd, equity_usd, global_pct, global_limit
                    ),
                    cooldown_secs: 3_600,
                });
            }

            let strategy_pct = baseline_dec("strategy_daily_loss_pct", dec_half());
            for (strategy_id, loss) in strategy_losses {
                let limit = equity_usd * strategy_pct / Decimal::from(100);
                if *loss > limit {
                    out.push(BreakerTrigger {
                        kind: RiskEventKind::StrategyDailyLoss,
                        severity: RiskSeverity::HardStop,
                        scope: RiskScope::Strategy,
                        scope_ref: strategy_id.clone(),
                        message: format!(
                            "策略 {} 日亏损 {} USDC 超过净值 {} USDC 的 {}%（上限 {} USDC），暂停该策略",
                            strategy_id, loss, equity_usd, strategy_pct, limit
                        ),
                        cooldown_secs: 3_600,
                    });
                }
            }
        }

        if peak_equity_usd > Decimal::ZERO {
            let drawdown_pct = baseline_dec("max_drawdown_pct", Decimal::from(5));
            let limit = peak_equity_usd * drawdown_pct / Decimal::from(100);
            let drawdown = peak_equity_usd - equity_usd;
            if drawdown > limit {
                out.push(BreakerTrigger {
                    kind: RiskEventKind::Drawdown,
                    severity: RiskSeverity::HardStop,
                    scope: RiskScope::Global,
                    scope_ref: "GLOBAL".to_string(),
                    message: format!(
                        "权益回撤 {} USDC 超过峰值 {} USDC 的 {}%（上限 {} USDC），全局暂停并重新评估",
                        drawdown, peak_equity_usd, drawdown_pct, limit
                    ),
                    cooldown_secs: 86_400,
                });
            }
        }

        out
    }

    /// 数据类熔断：盘口不完整 / RPC 失联 / 订单状态未知。
    pub fn data_breaker(
        &self,
        kind: RiskEventKind,
        scope: RiskScope,
        scope_ref: &str,
        message: &str,
    ) -> BreakerTrigger {
        let cooldown_secs = match kind {
            RiskEventKind::RpcUnhealthy => 60,
            RiskEventKind::BookInvalid => 30,
            RiskEventKind::OrderUnknown => 120,
            RiskEventKind::LedgerMismatch => 1_800,
            _ => 60,
        };
        BreakerTrigger {
            kind,
            severity: RiskSeverity::HardStop,
            scope,
            scope_ref: scope_ref.to_string(),
            message: message.to_string(),
            cooldown_secs,
        }
    }

    /// 恢复：清空对应作用域的窗口（链 ID 或交易对 / 策略标识）。
    pub fn clear(&mut self, scope_ref: &str) {
        if let Ok(chain_id) = scope_ref.parse::<u64>() {
            self.dex_reverts.remove(&chain_id);
        }
        self.hedge_attempts.remove(scope_ref);
    }

    /// 当前 15 分钟窗口内的 DEX 回滚次数（测试与前端用）。
    pub fn dex_revert_count(&self, chain_id: u64, now: DateTime<Utc>) -> usize {
        let Some(stored) = self.dex_reverts.get(&chain_id) else {
            return 0;
        };
        let mut window = stored.clone();
        Self::prune_dex(&mut window, now);
        window.len()
    }

    /// 最近窗口内的对冲异常次数（测试与前端用）。
    pub fn hedge_anomaly_count(&self, symbol: &str) -> usize {
        self.hedge_attempts
            .get(symbol)
            .map(|w| w.iter().filter(|a| **a).count())
            .unwrap_or(0)
    }

    fn prune_dex(window: &mut VecDeque<DateTime<Utc>>, now: DateTime<Utc>) {
        let cutoff = now - Duration::minutes(DEX_REVERT_WINDOW_MINS);
        while let Some(front) = window.front() {
            if *front < cutoff {
                window.pop_front();
            } else {
                break;
            }
        }
    }
}

fn dec_half() -> Decimal {
    Decimal::new(5, 1)
}

fn baseline_dec(key: &str, fallback: Decimal) -> Decimal {
    catalog::baseline()
        .into_iter()
        .find(|p| p.key == key)
        .map(|p| p.value)
        .unwrap_or(fallback)
}

fn baseline_usize(key: &str, fallback: usize) -> usize {
    baseline_dec(key, Decimal::from(fallback as u64))
        .to_usize()
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    fn t0() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn third_dex_revert_in_window_triggers() {
        let mut b = CircuitBreakers::new();
        let t0 = t0();
        assert!(b.record_dex_revert(1, t0).is_none());
        assert!(b.record_dex_revert(1, t0 + Duration::minutes(1)).is_none());
        let trigger = b
            .record_dex_revert(1, t0 + Duration::minutes(2))
            .expect("第 3 次回滚应触发熔断");
        assert_eq!(trigger.kind, RiskEventKind::DexReverts);
        assert_eq!(trigger.severity, RiskSeverity::HardStop);
        assert_eq!(trigger.scope, RiskScope::Chain);
        assert_eq!(trigger.scope_ref, "1");
        assert!(trigger.cooldown_secs >= 1_800);
        assert_eq!(b.dex_revert_count(1, t0 + Duration::minutes(2)), 3);

        // 不同链互不影响
        assert_eq!(b.dex_revert_count(2, t0 + Duration::minutes(2)), 0);
    }

    #[test]
    fn old_dex_reverts_outside_window_do_not_count() {
        let mut b = CircuitBreakers::new();
        let t0 = t0();
        let _ = b.record_dex_revert(1, t0);
        let later = t0 + Duration::minutes(16);
        assert!(b.record_dex_revert(1, later).is_none());
        assert!(b.record_dex_revert(1, later).is_none());
        assert_eq!(b.dex_revert_count(1, later), 2);
    }

    #[test]
    fn hedge_anomaly_triggers_on_second() {
        let mut b = CircuitBreakers::new();
        let t0 = t0();
        assert!(b.record_hedge_anomaly("ETHUSDT", t0).is_none());
        let trigger = b
            .record_hedge_anomaly("ETHUSDT", t0 + Duration::minutes(1))
            .expect("第 2 次对冲异常应触发熔断");
        assert_eq!(trigger.kind, RiskEventKind::HedgeAnomaly);
        assert_eq!(trigger.scope, RiskScope::Symbol);
        assert_eq!(trigger.scope_ref, "ETHUSDT");
        assert_eq!(b.hedge_anomaly_count("ETHUSDT"), 2);
        assert_eq!(b.hedge_anomaly_count("BTCUSDT"), 0);
    }

    #[test]
    fn hedge_window_keeps_only_latest_attempts() {
        let mut b = CircuitBreakers::new();
        let t0 = t0();
        let _ = b.record_hedge_anomaly("ETHUSDT", t0);
        let _ = b.record_hedge_anomaly("ETHUSDT", t0);
        // 再记录 10 次正常尝试，异常记录被挤出「最近 10 次」窗口
        for _ in 0..10 {
            // 异常仍在窗口内时继续返回触发是预期行为，这里只关心窗口裁剪
            let _ = b.record_hedge_attempt("ETHUSDT", false, t0);
        }
        assert_eq!(b.hedge_anomaly_count("ETHUSDT"), 0);
    }

    #[test]
    fn loss_and_drawdown_trigger() {
        let mut b = CircuitBreakers::new();

        let triggers = b.check_loss(dec("5000"), dec("5000"), dec("51"), &[]);
        assert!(
            triggers
                .iter()
                .any(|t| t.kind == RiskEventKind::DailyLoss
                    && t.scope == RiskScope::Global
                    && t.scope_ref == "GLOBAL"),
            "日亏损 51 > 5000*1% 应触发"
        );

        let triggers = b.check_loss(
            dec("5000"),
            dec("5000"),
            dec("0"),
            &[("dex_triple".to_string(), dec("30"))],
        );
        assert!(
            triggers
                .iter()
                .any(|t| t.kind == RiskEventKind::StrategyDailyLoss
                    && t.scope == RiskScope::Strategy
                    && t.scope_ref == "dex_triple"),
            "策略日亏损 30 > 5000*0.5% 应触发"
        );

        let triggers = b.check_loss(dec("4700"), dec("5000"), dec("0"), &[]);
        assert!(
            triggers
                .iter()
                .any(|t| t.kind == RiskEventKind::Drawdown && t.scope == RiskScope::Global),
            "回撤 300 > 5000*5% 应触发"
        );

        // 未超限时不触发
        let triggers = b.check_loss(dec("5000"), dec("5000"), dec("50"), &[]);
        assert!(triggers.is_empty(), "阈值边界不触发: {triggers:?}");
    }

    #[test]
    fn data_breaker_and_clear() {
        let mut b = CircuitBreakers::new();
        let trigger = b.data_breaker(
            RiskEventKind::BookInvalid,
            RiskScope::Symbol,
            "ETHUSDT",
            "盘口序号缺口",
        );
        assert_eq!(trigger.kind, RiskEventKind::BookInvalid);
        assert_eq!(trigger.severity, RiskSeverity::HardStop);
        assert!(trigger.cooldown_secs > 0);
        assert_eq!(trigger.message, "盘口序号缺口");

        let t0 = t0();
        for i in 0..3 {
            let _ = b.record_dex_revert(1, t0 + Duration::minutes(i));
        }
        assert_eq!(b.dex_revert_count(1, t0 + Duration::minutes(3)), 3);
        b.clear("1");
        assert_eq!(b.dex_revert_count(1, t0 + Duration::minutes(3)), 0);

        let _ = b.record_hedge_anomaly("ETHUSDT", t0);
        let _ = b.record_hedge_anomaly("ETHUSDT", t0);
        b.clear("ETHUSDT");
        assert_eq!(b.hedge_anomaly_count("ETHUSDT"), 0);
    }
}
