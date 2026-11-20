//! 交易前风控评估引擎（风控 §3–§9 的强制执行点）。
//!
//! 风控必须在后端执行：调用方只能通过 [`RiskEngine::evaluate`] 获取判定，
//! 硬限制（HARD_STOP / SOFT_BLOCK）不可被策略参数关闭。
//!
//! 覆盖规则（风控 §13）：全局 → 策略 → 链 → 交易所 → 交易对 → Token 逐级解析，
//! 每一级只允许比全局更严格，任何放宽尝试在 [`RiskEngine::set_override`] 被拒绝。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::domain::market::{RiskDecision, RiskViolation};
use crate::domain::risk::{violation, ParamContext, ParamOverride, RiskInput, RiskParam};
use crate::domain::types::{ParamDir, RiskScope, RiskSeverity, StrategyType};
use crate::risk::catalog;

/// 参数版本前缀；完整版本形如 `risk-v1.0.3`，覆盖变更时末位递增。
pub const VERSION_PREFIX: &str = "risk-v1.0";

/// 交易前风控引擎：参数目录 + 覆盖表 + 版本号。
pub struct RiskEngine {
    params: Vec<RiskParam>,
    overrides: Vec<ParamOverride>,
    /// 覆盖变更次数，同时作为参数版本末位。
    revision: u64,
}

impl Default for RiskEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskEngine {
    /// 载入 `catalog::baseline()`。
    pub fn new() -> Self {
        Self::with_params(catalog::baseline())
    }

    /// 使用自定义参数集合构造（迁移/审计回放用）。
    pub fn with_params(params: Vec<RiskParam>) -> Self {
        Self {
            params,
            overrides: Vec::new(),
            revision: 0,
        }
    }

    pub fn catalog(&self) -> &[RiskParam] {
        &self.params
    }

    pub fn find(&self, key: &str) -> Option<&RiskParam> {
        self.params.iter().find(|p| p.key == key)
    }

    pub fn overrides(&self) -> &[ParamOverride] {
        &self.overrides
    }

    /// 参数版本号，例如 `risk-v1.0.0`、`risk-v1.0.3`。
    pub fn version(&self) -> String {
        format!("{VERSION_PREFIX}.{}", self.revision)
    }

    /// 解析有效值：全局 → 策略 → 链 → 交易所 → 交易对 → Token 逐级覆盖，只允许更严格。
    pub fn effective(&self, key: &str, ctx: &ParamContext) -> Decimal {
        let Some(param) = self.find(key) else {
            return Decimal::ZERO;
        };
        let mut value = param.value;
        for scope in [
            RiskScope::Global,
            RiskScope::Strategy,
            RiskScope::Chain,
            RiskScope::Exchange,
            RiskScope::Symbol,
            RiskScope::Token,
        ] {
            for ov in self
                .overrides
                .iter()
                .filter(|o| o.key == key && o.scope == scope && override_applies(o, ctx))
            {
                value = stricter(param.dir, value, ov.value);
            }
        }
        value
    }

    /// 新增/更新覆盖。
    ///
    /// 校验：参数存在、作用域匹配、覆盖后不得比全局基线更宽松。
    pub fn set_override(&mut self, ov: ParamOverride) -> Result<(), String> {
        let param = self
            .find(&ov.key)
            .cloned()
            .ok_or_else(|| format!("未知风控参数: {}", ov.key))?;

        // 作用域：等于参数自身作用域；全局基线参数可在任意更细作用域收紧。
        let scope_ok = ov.scope == param.scope || param.scope == RiskScope::Global;
        if !scope_ok {
            return Err(format!(
                "作用域不匹配：参数 {} 的作用域为 {}，不接受 {} 覆盖",
                ov.key, param.scope, ov.scope
            ));
        }

        // 严格性：Max 型只能更小，Min 型只能更大。
        let stricter = match param.dir {
            ParamDir::Max => ov.value <= param.value,
            ParamDir::Min => ov.value >= param.value,
        };
        if !stricter {
            return Err(format!(
                "覆盖不得放宽硬限制：{} 的基线为 {} {}，覆盖值 {} {} 比基线更宽松",
                ov.key, param.value, param.unit, ov.value, param.unit
            ));
        }

        if let Some(min) = param.min {
            if ov.value < min {
                return Err(format!(
                    "覆盖值 {} 低于参数 {} 的允许下限 {}",
                    ov.value, ov.key, min
                ));
            }
        }
        if let Some(max) = param.max {
            if ov.value > max {
                return Err(format!(
                    "覆盖值 {} 高于参数 {} 的允许上限 {}",
                    ov.value, ov.key, max
                ));
            }
        }

        self.revision += 1;
        let mut ov = ov;
        ov.version = self.revision;
        if let Some(existing) = self.overrides.iter_mut().find(|o| {
            o.key == ov.key && o.scope == ov.scope && o.scope_ref == ov.scope_ref
        }) {
            *existing = ov;
        } else {
            self.overrides.push(ov);
        }
        Ok(())
    }

    /// 按 §3–§9 分组返回当前参数目录。
    pub fn groups(&self) -> Vec<catalog::ParamGroup> {
        catalog::group_params(self.params.clone())
    }

    /// 最低净利润门槛：max(2 USD, notional × 10bps)（仅 DEX 原子策略）。
    pub fn min_profit_for(&self, _strategy: StrategyType, notional: Decimal) -> Decimal {
        let (usd_key, bps_key, usd_default, bps_default) =
            ("dex_min_profit_usd", "dex_min_profit_bps", 2i64, 10i64);
        let usd = self
            .find(usd_key)
            .map(|p| p.value)
            .unwrap_or_else(|| Decimal::from(usd_default));
        let bps = self
            .find(bps_key)
            .map(|p| p.value)
            .unwrap_or_else(|| Decimal::from(bps_default));
        let by_bps = notional * bps / Decimal::from(10_000);
        usd.max(by_bps)
    }

    /// 交易前评估：返回 allowed + 全部违规项。
    ///
    /// 检查顺序固定（风控 §3–§9），每条违规的 `key` 与参数目录 key 对应，
    /// 便于前端直接跳转到对应参数。
    pub fn evaluate(
        &self,
        input: &RiskInput,
        controls: &crate::domain::risk::ControlState,
        now: DateTime<Utc>,
    ) -> RiskDecision {
        let ctx = self.context_for(input);
        let mut decision = RiskDecision {
            allowed: true,
            violations: Vec::new(),
            warnings: Vec::new(),
            checked_at: now,
            param_version: self.version(),
        };

        // 1. 暂停控制：全局 → 策略 → 链 → 交易所 → 交易对 → Token。
        let first_token = input.tokens.first().map(String::as_str);
        if let Some(msg) = controls.is_blocked(
            now,
            Some(input.strategy.as_str()),
            Some(input.chain_id),
            input.exchange.as_deref(),
            input.symbol.as_deref(),
            first_token,
        ) {
            record(
                &mut decision,
                violation(
                    "control_pause",
                    "暂停控制",
                    RiskSeverity::HardStop,
                    Decimal::ZERO,
                    Decimal::ONE,
                    "bool",
                    &msg,
                ),
            );
        }

        // 2. 数据质量：不完整或未知时默认拒绝交易（风控总原则 §1.4）。
        if !input.data_quality.is_executable() {
            record(
                &mut decision,
                violation(
                    "data_quality",
                    "数据质量",
                    RiskSeverity::HardStop,
                    Decimal::ONE,
                    Decimal::from(input.data_quality.rank() as i64),
                    "等级",
                    &format!(
                        "数据质量 {} 不可执行，拒绝生成交易计划",
                        input.data_quality
                    ),
                ),
            );
        }

        // 3. DEX 原子策略：报价区块差与报价年龄。
        if input.strategy.is_atomic() {
            let max_diff = self.value("dex_quote_block_diff_max", &ctx, Decimal::ONE);
            let diff = input.quote_block_number.abs_diff(input.head_block_number);
            if Decimal::from(diff) > max_diff {
                record(
                    &mut decision,
                    violation(
                        "dex_quote_block_diff_max",
                        "报价区块差上限",
                        self.param_sev("dex_quote_block_diff_max"),
                        max_diff,
                        Decimal::from(diff),
                        "区块",
                        &format!(
                            "报价区块 {} 与最新区块 {} 相差 {} 个区块，超过上限 {}",
                            input.quote_block_number, input.head_block_number, diff, max_diff
                        ),
                    ),
                );
            }

            let max_age = self.value("dex_quote_max_age_ms", &ctx, Decimal::from(3000));
            let age = Decimal::from(input.quote_age_ms.max(0));
            if age > max_age {
                record(
                    &mut decision,
                    violation(
                        "dex_quote_max_age_ms",
                        "DEX 报价最大年龄",
                        self.param_sev("dex_quote_max_age_ms"),
                        max_age,
                        age,
                        "ms",
                        &format!("报价年龄 {}ms 超过上限 {}ms，需重新报价", age, max_age),
                    ),
                );
            }
        }


        // 5. 净利润门槛：不使用理论毛利润。
        // 分层：净利润 ≤ 0 → HARD STOP；0 < 净利润 < 执行门槛 → WARNING（观察，近失）。
        let floor = self.min_profit_for(input.strategy, input.target_notional);
        if input.net_profit_usd < floor {
            let positive = input.net_profit_usd > Decimal::ZERO;
            record(
                &mut decision,
                violation(
                    "min_profit",
                    if positive {
                        "低于执行门槛（观察）"
                    } else {
                        "最低净利润"
                    },
                    if positive {
                        RiskSeverity::Warning
                    } else {
                        RiskSeverity::HardStop
                    },
                    floor,
                    input.net_profit_usd,
                    "USDC",
                    &format!(
                        "{} 保守净利润 {} USDC 低于执行门槛 {} USDC（缺口 {:.2} bps）",
                        input.strategy.label(),
                        input.net_profit_usd,
                        floor,
                        ((floor - input.net_profit_usd) / input.target_notional.max(Decimal::ONE)
                            * Decimal::from(10_000))
                        .to_f64()
                        .unwrap_or(0.0)
                    ),
                ),
            );
        }

        // 6. DEX Gas 约束：占毛利润比例与绝对上限。
        if input.strategy.is_atomic() {
            let ratio_max = self.value("gas_profit_ratio_max_pct", &ctx, Decimal::from(25));
            if input.gross_profit_usd > Decimal::ZERO {
                let ratio = input.gas_usd / input.gross_profit_usd * Decimal::from(100);
                if ratio > ratio_max {
                    record(
                        &mut decision,
                        violation(
                            "gas_profit_ratio_max_pct",
                            "Gas / 毛利润占比上限",
                            self.param_sev("gas_profit_ratio_max_pct"),
                            ratio_max,
                            ratio,
                            "%",
                            &format!(
                                "Gas 占毛利润 {}% 超过上限 {}%",
                                ratio, ratio_max
                            ),
                        ),
                    );
                }
            }

            let abs_max = self.value("gas_abs_max_usd", &ctx, Decimal::from(5));
            if input.gas_usd > abs_max {
                record(
                    &mut decision,
                    violation(
                        "gas_abs_max_usd",
                        "绝对 Gas 费用上限",
                        self.param_sev("gas_abs_max_usd"),
                        abs_max,
                        input.gas_usd,
                        "USDC",
                        &format!(
                            "Gas 费用 {} USDC 超过绝对上限 {} USDC",
                            input.gas_usd, abs_max
                        ),
                    ),
                );
            }
        }

        // 7. 价格冲击：DEX 与 CEX–DEX 各有 20 bps 上限。
        let impact_key = "dex_price_impact_max_bps";
        let impact_max = self.value(impact_key, &ctx, Decimal::from(20));
        if input.price_impact_bps > impact_max {
            record(
                &mut decision,
                violation(
                    impact_key,
                    "价格冲击上限",
                    self.param_sev(impact_key),
                    impact_max,
                    input.price_impact_bps,
                    "bps",
                    &format!(
                        "价格冲击 {} bps 超过上限 {} bps",
                        input.price_impact_bps, impact_max
                    ),
                ),
            );
        }

        // 8. 库存：任一场所/资产不足即拒绝。
        for need in &input.required_inventory {
            if !need.is_sufficient() {
                let shortfall = need.shortfall();
                record(
                    &mut decision,
                    violation(
                        "inventory",
                        "库存不足",
                        RiskSeverity::HardStop,
                        need.required,
                        shortfall,
                        &need.asset,
                        &format!(
                            "{} 的 {} 库存不足：需要 {}，可用 {}，缺口 {}",
                            need.venue, need.asset, need.required, need.available, shortfall
                        ),
                    ),
                );
            }
        }

        // 9. 闪电贷：费用必须已知，禁止硬编码。
        let is_flash = input
            .funding
            .as_ref()
            .map(|f| f.is_flash())
            .unwrap_or(false);
        if is_flash && !input.flash_fee_known {
            record(
                &mut decision,
                violation(
                    "flash_fee_must_be_known",
                    "闪电贷费用未知",
                    RiskSeverity::HardStop,
                    Decimal::ONE,
                    Decimal::ZERO,
                    "bool",
                    "闪电贷费用未知，拒绝执行（不得硬编码为固定收益成本）",
                ),
            );
        }

        // 10. 亏损与回撤：当前权益 = 已实现权益 + 未实现盈亏。
        let equity = input.equity_usd + input.unrealized_pnl_usd;

        let global_pct = self.value("global_daily_loss_pct", &ctx, Decimal::ONE);
        let global_limit = equity * global_pct / Decimal::from(100);
        if input.account_daily_loss_usd > global_limit {
            record(
                &mut decision,
                violation(
                    "global_daily_loss_pct",
                    "全局日亏损上限",
                    self.param_sev("global_daily_loss_pct"),
                    global_limit,
                    input.account_daily_loss_usd,
                    "USDC",
                    &format!(
                        "全局日亏损 {} USDC 超过净值 {} USDC 的 {}%（上限 {} USDC），触发全局暂停",
                        input.account_daily_loss_usd, equity, global_pct, global_limit
                    ),
                ),
            );
        }

        let strategy_pct = self.value("strategy_daily_loss_pct", &ctx, Decimal::new(5, 1));
        let strategy_limit = equity * strategy_pct / Decimal::from(100);
        if input.strategy_daily_loss_usd > strategy_limit {
            record(
                &mut decision,
                violation(
                    "strategy_daily_loss_pct",
                    "单策略日亏损上限",
                    self.param_sev("strategy_daily_loss_pct"),
                    strategy_limit,
                    input.strategy_daily_loss_usd,
                    "USDC",
                    &format!(
                        "策略 {} 日亏损 {} USDC 超过净值 {} 的 {}%（上限 {} USDC）",
                        input.strategy.label(),
                        input.strategy_daily_loss_usd,
                        equity,
                        strategy_pct,
                        strategy_limit
                    ),
                ),
            );
        }

        let drawdown_pct = self.value("max_drawdown_pct", &ctx, Decimal::from(5));
        if input.peak_equity_usd > Decimal::ZERO {
            let drawdown_limit =
                input.peak_equity_usd * drawdown_pct / Decimal::from(100);
            let drawdown = input.peak_equity_usd - equity;
            if drawdown > drawdown_limit {
                record(
                    &mut decision,
                    violation(
                        "max_drawdown_pct",
                        "最大权益回撤",
                        self.param_sev("max_drawdown_pct"),
                        drawdown_limit,
                        drawdown,
                        "USDC",
                        &format!(
                            "当前权益 {} USDC 相对峰值 {} USDC 回撤 {} USDC，超过 {}%（上限 {} USDC）",
                            equity, input.peak_equity_usd, drawdown, drawdown_pct, drawdown_limit
                        ),
                    ),
                );
            }
        }

        // 11. 单 Token 合并敞口（仅在权益 > 0 时可计算比例）。
        if equity > Decimal::ZERO {
            let exposure_pct = self.value("token_exposure_max_pct", &ctx, Decimal::from(10));
            let exposure_limit = equity * exposure_pct / Decimal::from(100);
            if input.target_notional > exposure_limit {
                record(
                    &mut decision,
                    violation(
                        "token_exposure_max_pct",
                        "单 Token 合并敞口上限",
                        self.param_sev("token_exposure_max_pct"),
                        exposure_limit,
                        input.target_notional,
                        "USDC",
                        &format!(
                            "目标名义 {} USDC 超过单 Token 敞口上限 {} USDC（净值 {} USDC × {}%）",
                            input.target_notional, exposure_limit, equity, exposure_pct
                        ),
                    ),
                );
            }
        }

        decision
    }

    /// 未来实盘专用检查（单笔 5% 净值上限等），Paper 阶段不参与放行判定。
    pub fn evaluate_live(&self, input: &RiskInput) -> Vec<RiskViolation> {
        let ctx = self.context_for(input);
        let equity = input.equity_usd + input.unrealized_pnl_usd;
        let mut out = Vec::new();
        if equity <= Decimal::ZERO {
            return out;
        }

        let single_pct = self.value("live_single_trade_max_pct", &ctx, Decimal::from(5));
        let single_limit = equity * single_pct / Decimal::from(100);
        if input.target_notional > single_limit {
            out.push(violation(
                "live_single_trade_max_pct",
                "实盘单笔净值上限（候选）",
                RiskSeverity::HardStop,
                single_limit,
                input.target_notional,
                "USDC",
                &format!(
                    "实盘候选单笔名义 {} USDC 超过净值 {} USDC 的 {}%（{} USDC）",
                    input.target_notional, equity, single_pct, single_limit
                ),
            ));
        }

        let exposure_pct = self.value("token_exposure_max_pct", &ctx, Decimal::from(10));
        let exposure_limit = equity * exposure_pct / Decimal::from(100);
        if input.target_notional > exposure_limit {
            out.push(violation(
                "token_exposure_max_pct",
                "单 Token 合并敞口上限",
                RiskSeverity::HardStop,
                exposure_limit,
                input.target_notional,
                "USDC",
                &format!(
                    "实盘名义 {} USDC 超过单 Token 敞口上限 {} USDC",
                    input.target_notional, exposure_limit
                ),
            ));
        }

        out
    }

    // ---- 内部辅助 ----

    fn context_for(&self, input: &RiskInput) -> ParamContext {
        ParamContext {
            strategy_id: Some(input.strategy.as_str().to_string()),
            chain_id: Some(input.chain_id),
            exchange: input.exchange.clone(),
            symbol: input.symbol.clone(),
            token: input.tokens.first().cloned(),
        }
    }

    /// 参数存在则返回覆盖解析后的有效值，否则回退到文档默认值。
    fn value(&self, key: &str, ctx: &ParamContext, fallback: Decimal) -> Decimal {
        if self.find(key).is_some() {
            self.effective(key, ctx)
        } else {
            fallback
        }
    }

    fn param_sev(&self, key: &str) -> RiskSeverity {
        self.find(key)
            .map(|p| p.severity)
            .unwrap_or(RiskSeverity::HardStop)
    }
}

/// 覆盖是否落在当前上下文中。
fn override_applies(ov: &ParamOverride, ctx: &ParamContext) -> bool {
    match ov.scope {
        RiskScope::Global => true,
        RiskScope::Strategy => ctx.strategy_id.as_deref() == Some(ov.scope_ref.as_str()),
        RiskScope::Chain => ctx
            .chain_id
            .map(|c| c.to_string() == ov.scope_ref)
            .unwrap_or(false),
        RiskScope::Exchange => ctx.exchange.as_deref() == Some(ov.scope_ref.as_str()),
        RiskScope::Symbol => ctx.symbol.as_deref() == Some(ov.scope_ref.as_str()),
        RiskScope::Token => ctx.token.as_deref() == Some(ov.scope_ref.as_str()),
    }
}

/// 只允许更严格：Max 取较小值，Min 取较大值。
fn stricter(dir: ParamDir, a: Decimal, b: Decimal) -> Decimal {
    match dir {
        ParamDir::Max => a.min(b),
        ParamDir::Min => a.max(b),
    }
}

/// 记录一条违规：WARNING 只进 warnings，其余直接拒绝。
fn record(decision: &mut RiskDecision, v: RiskViolation) {
    if v.severity == RiskSeverity::Warning {
        decision.warnings.push(v);
    } else {
        decision.allowed = false;
        decision.violations.push(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;
    use crate::domain::opportunity::{FundingSource, InventoryNeed};
    use crate::domain::risk::ControlState;
    use crate::domain::types::{DataQuality, ParamDir, RiskScope};

    fn engine() -> RiskEngine {
        RiskEngine::new()
    }

    fn ctx_chain(chain: u64) -> ParamContext {
        ParamContext {
            chain_id: Some(chain),
            ..Default::default()
        }
    }

    /// 一条可通过全部检查的 DEX 原子输入。
    fn dex_input() -> RiskInput {
        let mut i = RiskInput::zero(StrategyType::DexTriple);
        i.chain_id = 1;
        i.tokens = vec!["USDC".into(), "WETH".into()];
        i.target_notional = dec("400");
        i.net_profit_usd = dec("10");
        i.gross_profit_usd = dec("50");
        i.gas_usd = dec("2");
        i.price_impact_bps = dec("5");
        i.quote_age_ms = 100;
        i.quote_block_number = 1000;
        i.head_block_number = 1000;
        i.data_quality = DataQuality::Verified;
        i.equity_usd = dec("5000");
        i.peak_equity_usd = dec("5000");
        i
    }


    fn eval(input: &RiskInput) -> RiskDecision {
        engine().evaluate(input, &ControlState::new(), Utc::now())
    }

    fn assert_rejected(input: &RiskInput, expected_key: &str) -> RiskDecision {
        let d = eval(input);
        assert!(!d.allowed, "应被拒绝，实际放行；violations={:?}", d.violations);
        assert!(
            d.violations.iter().any(|v| v.key == expected_key),
            "缺少违规 key={expected_key}，实际 {:?}",
            d.violations.iter().map(|v| &v.key).collect::<Vec<_>>()
        );
        d
    }

    #[test]
    fn min_profit_floor_uses_bps_rule() {
        let e = engine();
        assert_eq!(
            e.min_profit_for(StrategyType::DexTriple, dec("1000")),
            dec("0.001")
        );
        assert_eq!(
            e.min_profit_for(StrategyType::DexTriple, dec("100000")),
            dec("0.1")
        );
    }

    #[test]
    fn baseline_inputs_are_allowed() {
        let d = eval(&dex_input());
        assert!(d.allowed, "DEX 基线应放行: {:?}", d.violations);
        assert!(d.violations.is_empty());
        assert_eq!(d.param_version, "risk-v1.0.0");
        assert!(d.checked_at.timestamp() > 0);
    }

    #[test]
    fn data_quality_rejects() {
        let mut i = dex_input();
        i.data_quality = DataQuality::Medium;
        assert_rejected(&i, "data_quality");
        assert!(!DataQuality::Low.is_executable());
    }

    #[test]
    fn stale_dex_quote_rejects() {
        let mut i = dex_input();
        i.quote_block_number = 1002;
        assert_rejected(&i, "dex_quote_block_diff_max");

        let mut i = dex_input();
        // 公共 RPC + M1 轮询把预算放宽到 8s（见 risk/catalog.rs 注释）
        i.quote_age_ms = 9000;
        assert_rejected(&i, "dex_quote_max_age_ms");
    }

    #[test]
    fn profit_gate_rejects() {
        // 净利 ≤ 0：硬拒绝。
        let mut i = dex_input();
        i.net_profit_usd = dec("-1");
        assert_rejected(&i, "min_profit");
    }

    #[test]
    fn positive_below_floor_is_watch_only() {
        // 净利 > 0 但低于执行门槛：只告警，不硬拒绝（供观察/近失统计）。
        let mut i = dex_input();
        i.net_profit_usd = dec("0.0001");
        let d = eval(&i);
        assert!(d.allowed, "正净利应放行到观察层");
        assert!(d
            .warnings
            .iter()
            .any(|v| v.key == "min_profit" && v.severity == RiskSeverity::Warning));
    }

    #[test]
    fn gas_limits_reject() {
        let mut i = dex_input();
        i.gas_usd = dec("20"); // 20/50 = 40% > 25%，且 20 > 5
        let d = assert_rejected(&i, "gas_abs_max_usd");
        assert!(d.violations.iter().any(|v| v.key == "gas_profit_ratio_max_pct"));

        let mut i = dex_input();
        i.gas_usd = dec("4"); // 4/50 = 8% <= 25%，且未超绝对上限 5
        assert!(eval(&i).allowed);
    }

    #[test]
    fn inventory_shortfall_rejects() {
        let mut i = dex_input();
        i.required_inventory = vec![InventoryNeed {
            venue: "dex_wallet".into(),
            asset: "WETH".into(),
            required: dec("1"),
            available: dec("0.4"),
        }];
        let d = assert_rejected(&i, "inventory");
        let v = d
            .violations
            .iter()
            .find(|v| v.key == "inventory")
            .unwrap();
        assert_eq!(v.actual, dec("0.6"));
        assert!(v.message.contains("dex_wallet"));
        assert!(v.message.contains("WETH"));
    }

    #[test]
    fn flash_fee_unknown_rejects() {
        let mut i = dex_input();
        i.funding = Some(FundingSource::FlashLoan {
            provider: "aave_v3".into(),
            token: "USDC".into(),
            amount: dec("400"),
            fee_bps: dec("9"),
            fee_amount: dec("0.36"),
        });
        i.flash_fee_known = false;
        assert_rejected(&i, "flash_fee_must_be_known");

        i.flash_fee_known = true;
        assert!(eval(&i).allowed);
    }

    #[test]
    fn price_impact_rejects() {
        let mut i = dex_input();
        i.price_impact_bps = dec("25");
        assert_rejected(&i, "dex_price_impact_max_bps");
    }

    #[test]
    fn loss_and_drawdown_reject() {
        let mut i = dex_input();
        i.account_daily_loss_usd = dec("51"); // 5000 * 1% = 50
        assert_rejected(&i, "global_daily_loss_pct");

        let mut i = dex_input();
        i.strategy_daily_loss_usd = dec("30"); // 5000 * 0.5% = 25
        assert_rejected(&i, "strategy_daily_loss_pct");

        let mut i = dex_input();
        i.equity_usd = dec("4700"); // 5000 峰值回撤 300 > 250
        assert_rejected(&i, "max_drawdown_pct");

        // 用 equity + unrealized_pnl 计算当前权益
        let mut i = dex_input();
        i.equity_usd = dec("5000");
        i.unrealized_pnl_usd = dec("-400"); // 当前 4600，回撤 400 > 250
        assert_rejected(&i, "max_drawdown_pct");
    }

    #[test]
    fn token_exposure_rejects() {
        let mut i = dex_input();
        i.target_notional = dec("600"); // 5000 * 10% = 500
        assert_rejected(&i, "token_exposure_max_pct");

        // 净值为 0 时不计算比例敞口
        let mut i = dex_input();
        i.equity_usd = Decimal::ZERO;
        i.peak_equity_usd = Decimal::ZERO;
        i.target_notional = dec("100000");
        let d = eval(&i);
        assert!(!d.violations.iter().any(|v| v.key == "token_exposure_max_pct"));
    }

    #[test]
    fn global_pause_rejects_with_reason() {
        let mut controls = ControlState::new();
        controls.set(
            RiskScope::Global,
            "GLOBAL",
            true,
            "手动停机演练",
            "test",
            None,
        );
        let d = engine().evaluate(&dex_input(), &controls, Utc::now());
        assert!(!d.allowed);
        let v = d
            .violations
            .iter()
            .find(|v| v.key == "control_pause")
            .expect("应产生 control_pause 违规");
        assert!(v.message.contains("全局暂停"), "message={}", v.message);
    }

    fn ov(key: &str, scope: RiskScope, scope_ref: &str, value: &str) -> ParamOverride {
        ParamOverride {
            key: key.to_string(),
            scope,
            scope_ref: scope_ref.to_string(),
            value: dec(value),
            updated_at: Utc::now(),
            updated_by: "tester".into(),
            reason: "收紧测试".into(),
            version: 0,
        }
    }

    #[test]
    fn override_must_not_loosen() {
        let mut e = engine();
        // 上限型：3 -> 5 放宽，必须拒绝
        let err = e
            .set_override(ov(
                "dex_quote_block_diff_max",
                RiskScope::Chain,
                "1",
                "5",
            ))
            .unwrap_err();
        assert!(err.contains("覆盖不得放宽硬限制"), "err={err}");
        assert!(e.overrides().is_empty());

        // 下限型：0.01 -> 0.001 放宽，必须拒绝
        let err = e
            .set_override(ov("dex_min_profit_bps", RiskScope::Global, "GLOBAL", "0.001"))
            .unwrap_err();
        assert!(err.contains("覆盖不得放宽硬限制"), "err={err}");
    }

    #[test]
    fn tighter_override_is_accepted_and_resolved() {
        let mut e = engine();
        assert_eq!(e.version(), "risk-v1.0.0");
        e.set_override(ov(
            "dex_quote_block_diff_max",
            RiskScope::Chain,
            "1",
            "0",
        ))
        .unwrap();
        assert_eq!(e.version(), "risk-v1.0.1");
        assert_eq!(
            e.effective("dex_quote_block_diff_max", &ctx_chain(1)),
            dec("0")
        );
        // 其它链不受影响
        assert_eq!(
            e.effective("dex_quote_block_diff_max", &ctx_chain(2)),
            dec("1")
        );

        // 全局参数可在策略级收紧
        e.set_override(ov(
            "dex_min_profit_usd",
            RiskScope::Strategy,
            "dex_triple",
            "3",
        ))
        .unwrap();
        let ctx = ParamContext {
            strategy_id: Some("dex_triple".into()),
            ..Default::default()
        };
        assert_eq!(e.effective("dex_min_profit_usd", &ctx), dec("3"));
        let ctx_other = ParamContext {
            strategy_id: Some("dex_cross".into()),
            ..Default::default()
        };
        assert_eq!(e.effective("dex_min_profit_usd", &ctx_other), dec("0"));
    }

    #[test]
    fn scope_mismatch_is_rejected() {
        let mut e = engine();
        // 链级参数不接受交易对覆盖
        let err = e
            .set_override(ov(
                "dex_quote_block_diff_max",
                RiskScope::Symbol,
                "ETHUSDT",
                "0",
            ))
            .unwrap_err();
        assert!(err.contains("作用域不匹配"), "err={err}");

        // 未知参数
        let err = e
            .set_override(ov("not_a_param", RiskScope::Global, "GLOBAL", "1"))
            .unwrap_err();
        assert!(err.contains("未知风控参数"), "err={err}");
    }

    #[test]
    fn first_override_update_does_not_duplicate() {
        let mut e = engine();
        e.set_override(ov("dex_min_profit_usd", RiskScope::Global, "GLOBAL", "3"))
            .unwrap();
        e.set_override(ov("dex_min_profit_usd", RiskScope::Global, "GLOBAL", "4"))
            .unwrap();
        assert_eq!(e.overrides().len(), 1);
        assert_eq!(e.version(), "risk-v1.0.2");
        assert_eq!(e.effective("dex_min_profit_usd", &ParamContext::default()), dec("4"));
    }

    #[test]
    fn live_checks_do_not_block_paper() {
        let e = engine();
        let mut i = dex_input();
        i.target_notional = dec("600"); // 超过单笔 5% = 250，但 Paper 的 evaluate 只看敞口
        let paper = e.evaluate(&i, &ControlState::new(), Utc::now());
        assert!(
            paper.violations.iter().any(|v| v.key == "token_exposure_max_pct"),
            "Paper 仍按 10% 敞口检查"
        );
        let live = e.evaluate_live(&i);
        assert!(live.iter().any(|v| v.key == "live_single_trade_max_pct"));
    }

    #[test]
    fn groups_are_exposed_by_engine() {
        let groups = engine().groups();
        assert_eq!(
            groups.iter().map(|g| g.items.len()).sum::<usize>(),
            catalog::baseline().len()
        );
    }

    #[test]
    fn param_direction_is_sane() {
        let e = engine();
        assert_eq!(
            e.find("global_daily_loss_pct").unwrap().dir,
            ParamDir::Max
        );
        assert_eq!(e.find("dex_min_profit_usd").unwrap().dir, ParamDir::Min);
    }
}
