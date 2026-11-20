//! 搜索配置：Token 白/黑名单与可在线修改的搜索参数。
//!
//! 持久化在 `data/search_config.json`；API 修改立即落盘，参数即时生效，
//! Token 列表变更在下一次 feed 重建/重启后生效（自动登记的新 Token 立即生效）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::domain::types::TokenStatus;

/// Token 准入状态。
pub const WHITELIST: &str = "WHITELIST";
pub const BLACKLIST: &str = "BLACKLIST";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenEntry {
    pub symbol: String,
    pub address: String,
    pub decimals: u32,
    /// WHITELIST / BLACKLIST
    pub status: String,
    /// 来源：config / auto / manual
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "config".into()
}

impl TokenEntry {
    pub fn is_whitelisted(&self) -> bool {
        self.status.eq_ignore_ascii_case(WHITELIST)
    }
    pub fn is_blacklisted(&self) -> bool {
        self.status.eq_ignore_ascii_case(BLACKLIST)
    }
}

/// 可在线修改的搜索条件。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchParams {
    pub min_pool_tvl_usd: f64,
    pub max_pool_share_pct: f64,
    pub min_input_usd: f64,
    pub max_input_usd: f64,
    pub max_pool_state_age_ms: i64,
    pub min_hops: usize,
    pub max_hops: usize,
    pub max_pools: usize,
    pub max_evaluate: usize,
    pub max_evaluate_ms: u64,
    pub notional_ladder: Vec<f64>,
    pub base_symbols: Vec<String>,
    // ---- 规模与算力预算 ----
    /// 单轮候选路线上限（图枚举阶段）。
    pub max_candidates: usize,
    /// 投入金额网格点数：越大越容易找到最优规模，代价是单路线定价变慢。
    pub grid_points: usize,
    /// 最优点细化迭代次数。
    pub refine_iters: u32,
    // ---- 时效 ----
    /// 机会有效期（秒）：超过即标记过期。
    pub ttl_sec: i64,
    /// 报价复用上限（毫秒）：低于该年龄的报价直接复用，不再重新询价。
    pub requote_age_ms: i64,
    /// 全量重建图谱的间隔（区块数）。
    pub full_rescan_blocks: u64,
    // ---- 成本与安全垫 ----
    /// Gas 价格覆盖（gwei）；0 表示使用链上实时值。
    pub gas_price_gwei: f64,
    /// 优先费（USD/笔）。
    pub priority_fee_usd: f64,
    /// 闪电贷费率（bps）。
    pub flash_fee_bps: f64,
    /// 延迟预留（bps，按名义金额计提）。
    pub delay_reserve_bps: f64,
    /// MEV / 竞争预留（bps）。
    pub mev_reserve_bps: f64,
    /// 利润折扣（%）：报价利润统一打折，作为安全边际。
    pub profit_discount_pct: f64,
    /// M1 与 M2 分叉复核的最大允许偏差（bps）。
    pub model_mismatch_max_bps: f64,
    // ---- 压力模型（M3）----
    /// 池状态不利扰动（bps）。
    pub stress_adverse_bps: f64,
    /// 压力场景下的 Gas 上调（%）。
    pub stress_gas_increase_pct: f64,
}

impl Default for SearchParams {
    fn default() -> Self {
        Self {
            min_pool_tvl_usd: 10_000.0,
            max_pool_share_pct: 10.0,
            min_input_usd: 100.0,
            max_input_usd: 5_000.0,
            max_pool_state_age_ms: 8_000,
            min_hops: 2,
            max_hops: 5,
            max_pools: 40,
            max_evaluate: 800,
            max_evaluate_ms: 1_200,
            notional_ladder: vec![100.0, 250.0, 400.0, 500.0, 1_000.0, 2_500.0, 5_000.0],
            base_symbols: vec!["USDC".into(), "USDT".into()],
            max_candidates: 100_000,
            grid_points: 30,
            refine_iters: 3,
            ttl_sec: 30,
            requote_age_ms: 12_000,
            full_rescan_blocks: 240,
            gas_price_gwei: 0.0,
            priority_fee_usd: 0.02,
            flash_fee_bps: 5.0,
            delay_reserve_bps: 2.0,
            mev_reserve_bps: 3.0,
            profit_discount_pct: 10.0,
            model_mismatch_max_bps: 50.0,
            stress_adverse_bps: 25.0,
            stress_gas_increase_pct: 50.0,
        }
    }
}

impl SearchParams {
    pub fn from_config(cfg: &Config) -> Self {
        let d = &cfg.dex_engine;
        let p = crate::strategy::dex_scan::DexScanParams::from_config(cfg);
        Self {
            min_pool_tvl_usd: d.min_pool_tvl_usd,
            max_pool_share_pct: d.max_pool_share_pct,
            min_input_usd: d.min_input_usd,
            max_input_usd: d.max_input_usd,
            max_pool_state_age_ms: d.max_pool_state_age_ms,
            min_hops: d.min_hops,
            max_hops: d.max_hops,
            max_pools: d.max_pools,
            max_evaluate: d.max_evaluate,
            max_evaluate_ms: d.max_evaluate_ms,
            notional_ladder: cfg.paper.notional_ladder.clone(),
            base_symbols: d.base_symbols.clone(),
            max_candidates: p.max_candidates,
            grid_points: p.grid_points,
            refine_iters: p.refine_iters,
            ttl_sec: p.ttl_sec,
            requote_age_ms: p.requote_age_ms,
            full_rescan_blocks: p.full_rescan_blocks,
            gas_price_gwei: 0.0,
            priority_fee_usd: f64_of(p.priority_fee_usd),
            flash_fee_bps: f64_of(p.flash_fee_bps),
            delay_reserve_bps: f64_of(p.delay_reserve_bps),
            mev_reserve_bps: f64_of(p.mev_reserve_bps),
            profit_discount_pct: f64_of(p.profit_discount_pct),
            model_mismatch_max_bps: f64_of(p.model_mismatch_max_bps),
            stress_adverse_bps: f64_of(p.stress.adverse_bps),
            stress_gas_increase_pct: f64_of(p.stress.gas_increase_pct),
        }
    }
}

/// Decimal → f64（配置展示用）。
fn f64_of(v: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    v.to_f64().unwrap_or(0.0)
}

impl SearchParams {
    /// 旧版配置文件（不含新增字段）升级：整组回填默认值，避免 0 值把费率/预算清零。
    pub fn upgrade_legacy(&mut self, cfg: &Config) {
        if self.ttl_sec > 0 {
            return;
        }
        let seed = Self::from_config(cfg);
        self.max_candidates = seed.max_candidates;
        self.grid_points = seed.grid_points;
        self.refine_iters = seed.refine_iters;
        self.ttl_sec = seed.ttl_sec;
        self.requote_age_ms = seed.requote_age_ms;
        self.full_rescan_blocks = seed.full_rescan_blocks;
        self.priority_fee_usd = seed.priority_fee_usd;
        self.flash_fee_bps = seed.flash_fee_bps;
        self.delay_reserve_bps = seed.delay_reserve_bps;
        self.mev_reserve_bps = seed.mev_reserve_bps;
        self.profit_discount_pct = seed.profit_discount_pct;
        self.model_mismatch_max_bps = seed.model_mismatch_max_bps;
        self.stress_adverse_bps = seed.stress_adverse_bps;
        self.stress_gas_increase_pct = seed.stress_gas_increase_pct;
    }
}

/// 完整搜索配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub tokens: Vec<TokenEntry>,
    pub params: SearchParams,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            tokens: Vec::new(),
            params: SearchParams::default(),
        }
    }
}

impl SearchConfig {
    fn path(workspace: &Path) -> PathBuf {
        workspace.join("data").join("search_config.json")
    }

    /// 读取配置；不存在时用 `cfg` 初始化并落盘。
    pub fn load(cfg: &Config, workspace: &Path) -> Self {
        let path = Self::path(workspace);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(mut parsed) = serde_json::from_str::<SearchConfig>(&text) {
                if parsed.tokens.is_empty() {
                    parsed.tokens = seed_tokens(cfg);
                }
                parsed.params.upgrade_legacy(cfg);
                return parsed;
            }
            tracing::warn!("search_config.json 解析失败，使用默认值: {}", path.display());
        }
        let mut seeded = Self {
            tokens: seed_tokens(cfg),
            params: SearchParams::from_config(cfg),
        };
        seeded.save(workspace);
        seeded
    }

    pub fn save(&self, workspace: &Path) {
        let path = Self::path(workspace);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    tracing::warn!("写入 {} 失败: {e}", path.display());
                }
            }
            Err(e) => tracing::warn!("序列化搜索配置失败: {e}"),
        }
    }

    /// 白名单 Token（供 feed 构造）。
    pub fn whitelist(&self) -> Vec<&TokenEntry> {
        self.tokens.iter().filter(|t| t.is_whitelisted()).collect()
    }

    pub fn blacklist(&self) -> Vec<&TokenEntry> {
        self.tokens.iter().filter(|t| t.is_blacklisted()).collect()
    }

    pub fn is_blacklisted(&self, symbol: &str) -> bool {
        self.tokens
            .iter()
            .any(|t| t.symbol.eq_ignore_ascii_case(symbol) && t.is_blacklisted())
    }

    /// 登记/更新 Token；`status` 为 WHITELIST/BLACKLIST。
    pub fn upsert(&mut self, symbol: &str, address: &str, decimals: u32, status: &str, source: &str) {
        let symbol = symbol.trim();
        let address = address.trim();
        if let Some(t) = self
            .tokens
            .iter_mut()
            .find(|t| !address.is_empty() && t.address.eq_ignore_ascii_case(address))
        {
            t.symbol = symbol.to_string();
            t.decimals = decimals;
            t.status = status.to_string();
            // 自动登记不覆盖人工/配置来源。
            if !source.is_empty() && !(source == "auto" && t.source != "auto") {
                t.source = source.to_string();
            }
            return;
        }
        self.tokens.push(TokenEntry {
            symbol: symbol.to_string(),
            address: address.to_string(),
            decimals,
            status: status.to_string(),
            source: source.to_string(),
        });
    }

    /// 移除 Token（白/黑名单同时删除）。
    pub fn remove(&mut self, symbol_or_address: &str) -> bool {
        let before = self.tokens.len();
        self.tokens.retain(|t| {
            !t.symbol.eq_ignore_ascii_case(symbol_or_address)
                && !t.address.eq_ignore_ascii_case(symbol_or_address)
        });
        self.tokens.len() != before
    }

    /// 白名单转成 `LiveTokenConfig`（feed 构造用）。
    pub fn token_configs(&self) -> Vec<crate::config::LiveTokenConfig> {
        self.whitelist()
            .into_iter()
            .map(|t| crate::config::LiveTokenConfig {
                symbol: t.symbol.clone(),
                address: t.address.clone(),
                decimals: Some(t.decimals),
                allowed_start: true,
                allowed_hop: true,
                allowed_flash: true,
                status: TokenStatus::Approved.as_str().to_string(),
            })
            .collect()
    }
}

fn seed_tokens(cfg: &Config) -> Vec<TokenEntry> {
    cfg.live
        .tokens
        .iter()
        .map(|t| TokenEntry {
            symbol: t.symbol.clone(),
            address: t.address.clone(),
            decimals: t.decimals.unwrap_or(18),
            status: WHITELIST.into(),
            source: "config".into(),
        })
        .collect()
}

impl SearchParams {
    /// 把在线修改的参数应用到扫描引擎。
    pub fn apply(&self, p: &mut crate::strategy::dex_scan::DexScanParams) {
        use rust_decimal::prelude::FromPrimitive;
        let d = |v: f64| rust_decimal::Decimal::from_f64_retain(v).unwrap_or_default();
        if self.min_pool_tvl_usd >= 0.0 {
            p.min_pool_tvl_usd = d(self.min_pool_tvl_usd);
        }
        if self.max_pool_share_pct > 0.0 {
            p.max_pool_share_pct = d(self.max_pool_share_pct);
        }
        if self.min_input_usd > 0.0 {
            p.min_input_usd = d(self.min_input_usd);
        }
        if self.max_input_usd >= self.min_input_usd {
            p.max_input_usd = d(self.max_input_usd);
        }
        if self.max_pool_state_age_ms > 0 {
            p.max_pool_state_age_ms = self.max_pool_state_age_ms;
        }
        p.min_hops = self.min_hops.max(2);
        p.max_hops = self.max_hops.max(p.min_hops);
        p.max_pools = self.max_pools.max(1);
        p.max_evaluate = self.max_evaluate.max(1);
        p.max_evaluate_ms = self.max_evaluate_ms.max(50);
        p.base_symbols = self.base_symbols.clone();
        if !self.notional_ladder.is_empty() {
            p.notional_ladder = self.notional_ladder.iter().map(|v| d(*v)).collect();
        }
        // ---- 规模与算力预算 ----
        if self.max_candidates > 0 {
            p.max_candidates = self.max_candidates;
        }
        if self.grid_points >= 5 {
            p.grid_points = self.grid_points;
        }
        if self.refine_iters >= 1 {
            p.refine_iters = self.refine_iters;
        }
        // ---- 时效 ----
        if self.ttl_sec >= 5 {
            p.ttl_sec = self.ttl_sec;
        }
        if self.requote_age_ms >= 500 {
            p.requote_age_ms = self.requote_age_ms;
        }
        if self.full_rescan_blocks >= 1 {
            p.full_rescan_blocks = self.full_rescan_blocks;
        }
        // ---- 成本与安全垫 ----
        if self.gas_price_gwei > 0.0 {
            p.gas_price_gwei = d(self.gas_price_gwei);
        }
        if self.priority_fee_usd >= 0.0 {
            p.priority_fee_usd = d(self.priority_fee_usd);
        }
        if self.flash_fee_bps >= 0.0 {
            p.flash_fee_bps = d(self.flash_fee_bps);
        }
        if self.delay_reserve_bps >= 0.0 {
            p.delay_reserve_bps = d(self.delay_reserve_bps);
        }
        if self.mev_reserve_bps >= 0.0 {
            p.mev_reserve_bps = d(self.mev_reserve_bps);
        }
        if self.profit_discount_pct >= 0.0 && self.profit_discount_pct <= 90.0 {
            p.profit_discount_pct = d(self.profit_discount_pct);
        }
        if self.model_mismatch_max_bps > 0.0 {
            p.model_mismatch_max_bps = d(self.model_mismatch_max_bps);
        }
        // ---- 压力模型 ----
        if self.stress_adverse_bps >= 0.0 {
            p.stress.adverse_bps = d(self.stress_adverse_bps);
        }
        if self.stress_gas_increase_pct >= 0.0 {
            p.stress.gas_increase_pct = d(self.stress_gas_increase_pct);
        }
    }
}
