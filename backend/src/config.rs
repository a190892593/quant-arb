//! 运行配置：从 `config/default.toml` 读取，支持环境变量覆盖。
//!
//! 第一阶段全部在本机运行（TDD §2.3），配置与数据目录均在工作区内，
//! 密钥类配置不允许出现在仓库中。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 顶层配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub data: DataConfig,
    pub paper: PaperConfig,
    pub feed: FeedConfig,
    pub cex: CexConfig,
    pub dex: DexConfig,
    pub chains: Vec<ChainSeed>,
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
    /// `demo`：本地合成市场（确定性、可离线）；`live`：读取实时公共行情。
    pub mode: String,
    pub seed: u64,
    pub tick_ms: u64,
}

impl Default for FeedConfig {
    fn default() -> Self {
        Self {
            mode: "demo".into(),
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
            cex: CexConfig::default(),
            dex: DexConfig::default(),
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
        let mut cfg = if path.exists() {
            let text = std::fs::read_to_string(&path)?;
            toml::from_str::<Config>(&text)?
        } else {
            Config::default()
        };

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
