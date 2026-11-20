//! 运行配置：从 `config/default.toml` 读取，支持环境变量覆盖。
//!
//! 第一阶段全部在本机运行（TDD §2.3），配置与数据目录均在工作区内，
//! 密钥类配置不允许出现在仓库中。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 真实链行情配置（第一阶段只支持单个 EVM 链的只读扫描）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveFeedConfig {
    pub chain_id: u64,
    pub rpc_primary: String,
    pub rpc_fallback: String,
    /// 免费 RPC 池：主节点失败时轮换，全部不需要 Key。
    pub rpc_pool: Vec<String>,
    /// 链状态轮询间隔（毫秒）。
    pub refresh_ms: u64,
    pub max_pools: usize,
    /// V3 tick 位图检索半径（按 256 位字计，1 字 = 256×tick_spacing 个 tick）。
    pub tick_word_radius: i32,
    pub max_ticks_per_pool: usize,
    /// 每轮最多加载 tick 的高 TVL 池数量（控制免费 RPC 调用量）。
    #[serde(default = "default_tick_top_pools")]
    pub tick_top_pools: usize,
    pub tick_refresh_ms: i64,
    pub tokens: Vec<LiveTokenConfig>,
    pub v2_factories: Vec<V2FactoryConfig>,
    pub v3_factories: Vec<V3FactoryConfig>,
    /// 链上报价交叉验证使用的路由/报价器。
    pub v2_routers: Vec<V2RouterConfig>,
    pub quoter_v3: String,
    pub extra_pools: Vec<ExtraPoolConfig>,
}

fn default_tick_top_pools() -> usize {
    8
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LiveTokenConfig {
    pub symbol: String,
    pub address: String,
    /// 可省略：启动时用链上 decimals() 校验/补齐。
    pub decimals: Option<u32>,
    pub allowed_start: bool,
    pub allowed_hop: bool,
    pub allowed_flash: bool,
    /// APPROVED / RESTRICTED / OBSERVING / UNKNOWN / REJECTED
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct V2FactoryConfig {
    pub protocol_id: String,
    pub address: String,
    pub fee_bps: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct V3FactoryConfig {
    pub protocol_id: String,
    pub address: String,
    pub fee_tiers: Vec<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct V2RouterConfig {
    pub protocol_id: String,
    pub router: String,
}

/// 额外池（Curve/Balancer 等无法通过工厂 getPair/getPool 枚举的池）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtraPoolConfig {
    pub protocol_id: String,
    pub address: String,
    /// UNISWAP_V2 / UNISWAP_V3 / STABLESWAP / WEIGHTED
    pub kind: String,
    pub fee_bps: i64,
    pub token0: String,
    pub token1: String,
    pub decimals0: u32,
    pub decimals1: u32,
    pub tick_spacing: Option<i32>,
}

impl Default for LiveFeedConfig {
    fn default() -> Self {
        Self {
            chain_id: 42_161,
            rpc_primary: "https://arb1.arbitrum.io/rpc".into(),
            rpc_fallback: "https://arbitrum-one.publicnode.com".into(),
            rpc_pool: vec![
                "https://arbitrum-one.publicnode.com".into(),
                "https://1rpc.io/arb".into(),
                "https://arbitrum.drpc.org".into(),
            ],
            refresh_ms: 1_200,
            max_pools: 128,
            tick_word_radius: 2,
            max_ticks_per_pool: 320,
            tick_top_pools: 8,
            tick_refresh_ms: 20_000,
            tokens: vec![
                LiveTokenConfig {
                    symbol: "USDC".into(),
                    address: "0xaf88d065e77c8cC2239327C5EDb3A432268e5831".into(),
                    decimals: Some(6),
                    allowed_start: true,
                    allowed_hop: true,
                    allowed_flash: true,
                    status: "APPROVED".into(),
                },
                LiveTokenConfig {
                    symbol: "USDT".into(),
                    address: "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9".into(),
                    decimals: Some(6),
                    allowed_start: true,
                    allowed_hop: true,
                    allowed_flash: true,
                    status: "APPROVED".into(),
                },
                LiveTokenConfig {
                    symbol: "WETH".into(),
                    address: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1".into(),
                    decimals: Some(18),
                    allowed_start: true,
                    allowed_hop: true,
                    allowed_flash: true,
                    status: "APPROVED".into(),
                },
                LiveTokenConfig {
                    symbol: "WBTC".into(),
                    address: "0x2f2a2543B76A4166549F7aaB2e75Bef0aefC5B0f".into(),
                    decimals: Some(8),
                    allowed_start: true,
                    allowed_hop: true,
                    allowed_flash: true,
                    status: "APPROVED".into(),
                },
                LiveTokenConfig {
                    symbol: "ARB".into(),
                    address: "0x912CE59144191C1204E64559FE8253a0e49E6548".into(),
                    decimals: Some(18),
                    allowed_start: true,
                    allowed_hop: true,
                    allowed_flash: true,
                    status: "APPROVED".into(),
                },
            ],
            v2_factories: vec![
                V2FactoryConfig {
                    protocol_id: "uniswap-v2".into(),
                    address: "0xf1D7CC64Fb4452F05c498126312eBE29f30Fbcf9".into(),
                    fee_bps: 30,
                },
                V2FactoryConfig {
                    protocol_id: "sushi-v2".into(),
                    address: "0xc35DADB65012eC5796536bD9864eD8773aBc74C4".into(),
                    fee_bps: 30,
                },
            ],
            v3_factories: vec![V3FactoryConfig {
                protocol_id: "uniswap-v3".into(),
                address: "0x1F98431c8aD98523631AE4a59f267346ea31F984".into(),
                fee_tiers: vec![100, 500, 3000, 10000],
            }],
            v2_routers: vec![
                V2RouterConfig {
                    protocol_id: "uniswap-v2".into(),
                    router: "0x4752ba5DBc23f44D87826276BF6Fd6b1C372aD24".into(),
                },
                V2RouterConfig {
                    protocol_id: "sushi-v2".into(),
                    router: "0x1b02dA8Cb0d097eB8D57A175b88c7D8b47997506".into(),
                },
            ],
            quoter_v3: "0x61fFE014bA17989E7435cF6cB21bF9697530B21e".into(),
            extra_pools: Vec::new(),
        }
    }
}

/// 顶层配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub data: DataConfig,
    pub paper: PaperConfig,
    pub feed: FeedConfig,
    pub live: LiveFeedConfig,
    pub cex: CexConfig,
    pub dex: DexConfig,
    pub dex_engine: DexEngineConfig,
    pub m2: M2Config,
    pub discovery: DiscoveryConfig,
    pub chains: Vec<ChainSeed>,
}

/// 全链 Token/池自动发现配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    pub enabled: bool,
    /// 两次发现扫描的最小间隔（秒）。
    pub interval_sec: u64,
    /// Uniswap V3 工厂地址。
    pub factory_v3: String,
    /// V2 风格工厂（PairCreated），例如 Sushi/Camelot。
    pub factories_v2: Vec<String>,
    /// 首次启动回看区块数。
    pub lookback_blocks: u64,
    /// 显式起始区块（优先于回看）。
    pub start_block: Option<u64>,
    /// 入池 TVL 门槛（USD）。
    pub min_tvl_usd: f64,
    /// 单轮最多新增池数。
    pub max_new_pools: usize,
    /// 发现网络（GeckoTerminal 网络标识，如 arbitrum / base）。
    pub network: String,
    /// 发现池总数上限。
    pub max_total_pools: usize,
    /// CoinGecko Demo/Pro API Key；留空则走 GeckoTerminal 公共端点（30 次/分）。
    pub coingecko_api_key: String,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_sec: 300,
            factory_v3: "0x1F98431c8aD98523631AE4a59f267346ea31F984".into(),
            factories_v2: vec![
                "0xc35DADB65012eC5796536bD9864eD8773aBc74C4".into(),
                "0x6EcCab422D763aC031210895C81787E87B43A652".into(),
            ],
            lookback_blocks: 5_000,
            start_block: None,
            min_tvl_usd: 10_000.0,
            max_new_pools: 50,
            network: "arbitrum".into(),
            max_total_pools: 200,
            coingecko_api_key: String::new(),
        }
    }
}

/// M2（REVM 分叉执行）判定配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct M2Config {
    /// 是否在扫描中执行 M2 复核。
    pub enabled: bool,
    /// M2 与 M1 输出的最大允许偏差（bps）。
    pub tolerance_bps: f64,
    pub gas_limit: u64,
    /// QuoterV2 地址（Arbitrum 主网默认）。
    pub quoter: String,
    /// M2 专用 RPC：必须支持按历史区块读取存储（公共 RPC 常拒绝 archive 请求）。
    pub rpc: String,
    /// 两次 M2 复核的最小间隔（毫秒）；REVM 分叉读取较慢，避免拖慢每次扫描。
    pub every_ms: i64,
}

impl Default for M2Config {
    fn default() -> Self {
        Self {
            enabled: true,
            tolerance_bps: 50.0,
            gas_limit: 3_000_000,
            quoter: "0x61fFE014bA17989E743c5F6cB21bF9697530B21e".into(),
            rpc: "https://arb1.arbitrum.io/rpc".into(),
            every_ms: 15_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// 前端静态目录（相对仓库根或绝对路径）。
    pub ui_dir: String,
    /// 是否启用 CORS（本地开发时前端可能由其他端口提供）。
    pub cors: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8791,
            ui_dir: "frontend/dist".into(),
            cors: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DataConfig {
    /// 数据目录（相对仓库根）。
    pub dir: String,
    pub db_file: String,
    /// 保留的机会记录上限（避免本地库无限增长）。
    pub keep_opportunities: usize,
    pub keep_risk_events: usize,
}

impl Default for DataConfig {
    fn default() -> Self {
        Self {
            dir: "data".into(),
            db_file: "arbx.db".into(),
            keep_opportunities: 2_000,
            keep_risk_events: 5_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PaperConfig {
    pub start_equity: f64,
    /// DEX 与 CEX 的资金分配（50:50 为对比基准，不是最优结论）。
    pub dex_share: f64,
    pub scan_interval_ms: u64,
    pub opportunity_ttl_sec: i64,
    /// 自动执行机会（Paper）。关闭后只能手工触发，便于观察扫描。
    pub auto_execute: bool,
    /// 每轮扫描后触发一次执行尝试的概率上限（0–1），避免每轮都成交。
    pub execute_probability: f64,
    /// 单笔目标名义金额档位（USDC）。
    pub notional_ladder: Vec<f64>,
    /// 同时保持的未完成执行数量上限。
    pub max_inflight: usize,
}

impl Default for PaperConfig {
    fn default() -> Self {
        Self {
            start_equity: 5_000.0,
            dex_share: 0.5,
            scan_interval_ms: 1_500,
            opportunity_ttl_sec: 30,
            auto_execute: true,
            execute_probability: 0.5,
            notional_ladder: vec![250.0, 500.0, 1_000.0, 2_500.0],
            max_inflight: 3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedConfig {
    /// 运行时只接受 `live`（真实链数据）。`demo` 仅供单元测试直接构造市场使用。
    pub mode: String,
    pub seed: u64,
    pub tick_ms: u64,
}

impl Default for FeedConfig {
    fn default() -> Self {
        Self {
            // 默认必须是真实链数据；演示行情只能通过显式配置开启（且运行时拒绝）。
            mode: "live".into(),
            seed: 20_261_119,
            tick_ms: 1_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CexConfig {
    pub exchange: String,
    pub symbols: Vec<String>,
    /// 纸面费率（bps）。真实费率需账户接口，第一阶段以配置显式声明。
    pub taker_bps: f64,
    pub maker_bps: f64,
    /// 是否把配置费率视为「已确认」。未确认的费率不允许生成可执行机会。
    pub fee_confirmed: bool,
    /// 盘口档位数（合成市场）。
    pub book_levels: usize,
}

impl Default for CexConfig {
    fn default() -> Self {
        Self {
            exchange: "binance".into(),
            symbols: vec!["ETHUSDT".into(), "BTCUSDT".into()],
            taker_bps: 10.0,
            maker_bps: 8.0,
            fee_confirmed: true,
            book_levels: 12,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DexConfig {
    /// 单跳 Gas（units）与 Gas 价格，用于成本模型。
    pub gas_price_gwei: f64,
    pub native_price_usd: f64,
    pub gas_per_hop: u64,
    /// 闪电贷费率（bps），必须来自实际协议数据；演示模式使用显式配置值。
    pub flash_loan_fee_bps: f64,
    pub flash_provider: String,
}

impl Default for DexConfig {
    fn default() -> Self {
        Self {
            gas_price_gwei: 0.02,
            native_price_usd: 2_800.0,
            gas_per_hop: 130_000,
            flash_loan_fee_bps: 5.0,
            flash_provider: "aave-v3".into(),
        }
    }
}

/// DEX 多池多跳扫描引擎（PRD §4–§7）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DexEngineConfig {
    /// 是否启用新引擎（关闭时回退到旧版三角/跨池扫描）。
    pub enabled: bool,
    pub min_hops: usize,
    pub max_hops: usize,
    pub max_pools: usize,
    pub max_candidates: usize,
    /// 单轮最多完整定价的候选数。
    pub max_evaluate: usize,
    /// 定价阶段硬预算（毫秒）。
    pub max_evaluate_ms: u64,
    pub max_visits: u64,
    pub max_elapsed_ms: u64,
    pub min_input_usd: f64,
    pub max_input_usd: f64,
    pub min_pool_tvl_usd: f64,
    pub max_pool_state_age_ms: i64,
    /// 单池最多吃掉「一侧储备价值」的百分比（容量约束）。
    pub max_pool_share_pct: f64,
    pub grid_points: usize,
    pub refine_iters: u32,
    /// M3 压力模拟：池状态不利扰动（bps）与 Gas / 闪电贷费用上调（%）。
    pub stress_bps: f64,
    pub stress_gas_pct: f64,
    pub stress_flash_pct: f64,
    pub model_mismatch_max_bps: f64,
    /// 增量扫描：报价超过该年龄强制重算（毫秒）。
    pub requote_age_ms: i64,
    /// 每 N 个区块强制重建一次路线集合。
    pub full_rescan_blocks: u64,
    pub gas_executor_base: u64,
    pub gas_executor_per_hop: u64,
    pub gas_transfer_per_hop: u64,
    pub gas_flash_loan: u64,
    pub gas_approve: u64,
    pub base_symbols: Vec<String>,
}

impl Default for DexEngineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_hops: 2,
            max_hops: 5,
            max_pools: 5,
            max_candidates: 100_000,
            max_evaluate: 800,
            max_evaluate_ms: 1_200,
            max_visits: 200_000,
            max_elapsed_ms: 250,
            min_input_usd: 50.0,
            max_input_usd: 25_000.0,
            min_pool_tvl_usd: 100_000.0,
            max_pool_state_age_ms: 3_000,
            max_pool_share_pct: 10.0,
            grid_points: 14,
            refine_iters: 16,
            stress_bps: 25.0,
            stress_gas_pct: 50.0,
            stress_flash_pct: 50.0,
            model_mismatch_max_bps: 50.0,
            requote_age_ms: 800,
            full_rescan_blocks: 30,
            gas_executor_base: 45_000,
            gas_executor_per_hop: 12_000,
            gas_transfer_per_hop: 30_000,
            gas_flash_loan: 90_000,
            gas_approve: 50_000,
            base_symbols: vec!["USDC".into(), "USDT".into()],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainSeed {
    pub chain_id: u64,
    pub name: String,
    pub native: String,
    pub rpc_primary: String,
    pub rpc_secondary: String,
    pub block_time_ms: u64,
    pub stage: String,
    pub gas_model: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            data: DataConfig::default(),
            paper: PaperConfig::default(),
            feed: FeedConfig::default(),
            live: LiveFeedConfig::default(),
            cex: CexConfig::default(),
            dex: DexConfig::default(),
            dex_engine: DexEngineConfig::default(),
            m2: M2Config::default(),
            discovery: DiscoveryConfig::default(),
            chains: vec![
                ChainSeed {
                    chain_id: 42161,
                    name: "Arbitrum".into(),
                    native: "ETH".into(),
                    rpc_primary: "https://arb1.arbitrum.io/rpc".into(),
                    rpc_secondary: "https://arbitrum.llamarpc.com".into(),
                    block_time_ms: 260,
                    stage: "PAPER_READY".into(),
                    gas_model: "L2 单向计费".into(),
                },
                ChainSeed {
                    chain_id: 8453,
                    name: "Base".into(),
                    native: "ETH".into(),
                    rpc_primary: "https://mainnet.base.org".into(),
                    rpc_secondary: "https://base.llamarpc.com".into(),
                    block_time_ms: 2_000,
                    stage: "PAPER_READY".into(),
                    gas_model: "OP Stack L2 计费".into(),
                },
                ChainSeed {
                    chain_id: 1,
                    name: "Ethereum".into(),
                    native: "ETH".into(),
                    rpc_primary: "https://cloudflare-eth.com".into(),
                    rpc_secondary: "https://ethereum-rpc.publicnode.com".into(),
                    block_time_ms: 12_000,
                    stage: "DATA_VALIDATED".into(),
                    gas_model: "EIP-1559 base + priority".into(),
                },
            ],
        }
    }
}

impl Config {
    /// 从工作区根目录加载：`config/default.toml`，缺失则用默认值。
    pub fn load(workspace: &Path) -> anyhow::Result<Self> {
        let path = workspace.join("config").join("default.toml");
        let mut value: toml::Value = if path.exists() {
            let text = std::fs::read_to_string(&path)?;
            toml::from_str::<toml::Value>(&text)?
        } else {
            toml::Value::try_from(Config::default())?
        };
        // 配置档叠加：ARBX_PROFILE=base 时用 config/base.toml 覆盖默认配置。
        if let Ok(profile) = std::env::var("ARBX_PROFILE") {
            let profile = profile.trim();
            if !profile.is_empty() {
                let ppath = workspace.join("config").join(format!("{profile}.toml"));
                if ppath.exists() {
                    let text = std::fs::read_to_string(&ppath)?;
                    let overlay: toml::Value = toml::from_str(&text)?;
                    merge_toml(&mut value, overlay);
                } else {
                    anyhow::bail!("配置档不存在: {}", ppath.display());
                }
            }
        }
        let mut cfg: Config = value
            .try_into()
            .map_err(|e| anyhow::anyhow!("配置解析失败: {e}"))?;

        // 环境变量覆盖（仅覆盖有限项，避免隐式行为）
        if let Ok(v) = std::env::var("ARBX_PORT") {
            if let Ok(p) = v.parse::<u16>() {
                cfg.server.port = p;
            }
        }
        if let Ok(v) = std::env::var("ARBX_HOST") {
            cfg.server.host = v;
        }
        if let Ok(v) = std::env::var("ARBX_DATA_DIR") {
            cfg.data.dir = v;
        }
        if let Ok(v) = std::env::var("ARBX_FEED_MODE") {
            cfg.feed.mode = v;
        }
        if let Ok(v) = std::env::var("ARBX_UI_DIR") {
            cfg.server.ui_dir = v;
        }
        Ok(cfg)
    }

    pub fn data_dir(&self, workspace: &Path) -> PathBuf {
        resolve(workspace, &self.data.dir)
    }

    pub fn db_path(&self, workspace: &Path) -> PathBuf {
        self.data_dir(workspace).join(&self.data.db_file)
    }

    pub fn ui_dir(&self, workspace: &Path) -> PathBuf {
        resolve(workspace, &self.server.ui_dir)
    }

    pub fn dex_notional(&self) -> f64 {
        self.paper.start_equity * self.paper.dex_share
    }

    pub fn cex_notional(&self) -> f64 {
        self.paper.start_equity * (1.0 - self.paper.dex_share)
    }
}

/// 相对路径按工作区解析；绝对路径原样使用。
fn resolve(workspace: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_risk_baseline() {
        let c = Config::default();
        assert_eq!(c.paper.start_equity, 5_000.0);
        assert_eq!(c.paper.dex_share, 0.5);
        assert_eq!(c.dex_notional(), 2_500.0);
        assert_eq!(c.dex_notional() + c.cex_notional(), 5_000.0);
        assert_eq!(c.paper.opportunity_ttl_sec, 30);
    }

    #[test]
    fn parses_toml_and_env_override() {
        let text = r#"
[server]
port = 9100

[paper]
start_equity = 5000.0
dex_share = 0.6
"#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.server.port, 9100);
        assert_eq!(cfg.paper.dex_share, 0.6);
        // 未指定的字段走默认
        assert_eq!(cfg.cex.exchange, "binance");
        assert_eq!(cfg.chains.len(), 3);
    }
}


fn merge_toml(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_t), toml::Value::Table(over)) => {
            for (k, v) in over {
                match base_t.get_mut(&k) {
                    Some(slot) => merge_toml(slot, v),
                    None => {
                        base_t.insert(k, v);
                    }
                }
            }
        }
        (slot, over) => *slot = over,
    }
}
