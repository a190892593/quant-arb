//! 扫描数据源注册表：启用状态、优先级、Key、限速与切换策略。
//!
//! 持久化在 `data/source_config.json`；发现任务每轮读取，修改后无需重启。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceEntry {
    pub id: String,
    pub name: String,
    /// gecko / coingecko / dexscreener / rpc / data / security / explorer / cex
    pub kind: String,
    pub enabled: bool,
    /// 数值越小优先级越高（priority 策略使用）。
    pub priority: i64,
    pub api_key: String,
    pub url: String,
    pub rate_limit_per_min: i64,
    #[serde(default)]
    pub last_status: String,
    #[serde(default)]
    pub last_used_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceConfig {
    /// round_robin / priority / random
    pub strategy: String,
    pub interval_sec: u64,
    pub sources: Vec<SourceEntry>,
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            strategy: "round_robin".into(),
            interval_sec: 1800,
            sources: default_sources(),
        }
    }
}

fn s(
    id: &str,
    name: &str,
    kind: &str,
    priority: i64,
    rate: i64,
    url: &str,
) -> SourceEntry {
    SourceEntry {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        enabled: true,
        priority,
        api_key: String::new(),
        url: url.into(),
        rate_limit_per_min: rate,
        last_status: String::new(),
        last_used_at: String::new(),
    }
}

pub const KEY_REQUIRED: &[&str] = &[
    "coingecko_top",
    "coingecko_top2",
    "goplus",
    "arbiscan",
    "alchemy_rpc",
    "infura_rpc",
    "quicknode_rpc",
    "ankr_rpc",
    "thegraph",
    "coinpaprika",
    "cryptocompare",
    "moralis",
    "covalent",
];

/// 完全免 Key 的数据源集合。
pub const FREE_IDS: &[&str] = &[
    "gecko_top",
    "gecko_top2",
    "gecko_new",
    "gecko_trending",
    "dexscreener",
    "defillama",
    "blockscout",
    "honeypot",
    "pyth",
    "curve",
    "balancer",
    "public_rpc",
    "binance_cex",
    "sourcify",
];

fn default_sources() -> Vec<SourceEntry> {
    let mut list = vec![
        s(
            "gecko_top",
            "GeckoTerminal 高 TVL 池",
            "gecko",
            10,
            30,
            "https://api.geckoterminal.com/api/v2/networks/arbitrum/pools?page=1&sort=h24_volume_usd_desc",
        ),
        s(
            "gecko_top2",
            "GeckoTerminal 高 TVL 池 第 2 页",
            "gecko",
            11,
            30,
            "https://api.geckoterminal.com/api/v2/networks/arbitrum/pools?page=2&sort=h24_volume_usd_desc",
        ),
        s(
            "gecko_new",
            "GeckoTerminal 新池",
            "gecko",
            12,
            30,
            "https://api.geckoterminal.com/api/v2/networks/arbitrum/new_pools",
        ),
        s(
            "gecko_trending",
            "GeckoTerminal 热门池",
            "gecko",
            13,
            30,
            "https://api.geckoterminal.com/api/v2/networks/arbitrum/trending_pools",
        ),
        s(
            "coingecko_top",
            "CoinGecko Onchain 高 TVL 池",
            "coingecko",
            20,
            100,
            "https://api.coingecko.com/api/v3/onchain/networks/arbitrum/pools?page=1&sort=h24_volume_usd_desc",
        ),
        s(
            "coingecko_top2",
            "CoinGecko Onchain 第 2 页",
            "coingecko",
            21,
            100,
            "https://api.coingecko.com/api/v3/onchain/networks/arbitrum/pools?page=2&sort=h24_volume_usd_desc",
        ),
        s("dexscreener", "DexScreener Boost→Pairs", "dexscreener", 30, 300, "https://api.dexscreener.com/token-boosts/latest/v1"),
        s("defillama", "DefiLlama TVL/池", "data", 40, 60, "https://api.llama.fi"),
        s("goplus", "GoPlus Token 安全", "security", 50, 30, "https://api.gopluslabs.io"),
        s("arbiscan", "Arbiscan 浏览器 API", "explorer", 60, 180, "https://api.arbiscan.io/api"),
        s("alchemy_rpc", "Alchemy RPC（免费档）", "rpc", 70, 300, "https://arb-mainnet.g.alchemy.com/v2"),
        s("infura_rpc", "Infura RPC（免费档）", "rpc", 71, 300, "https://arbitrum-mainnet.infura.io/v3"),
        s("public_rpc", "Arbitrum 公共 RPC", "rpc", 80, 60, "https://arb1.arbitrum.io/rpc"),
        s("binance_cex", "Binance 公共行情", "cex", 90, 1200, "https://api.binance.com"),
        s("thegraph", "The Graph Studio（子图）", "data", 100, 60, "https://api.studio.thegraph.com/query"),
        s("blockscout", "Blockscout 浏览器", "explorer", 101, 60, "https://arbitrum.blockscout.com/api/v2"),
        s("honeypot", "Honeypot.is 蜜罐检测", "security", 102, 30, "https://api.honeypot.is/v2/IsHoneypot"),
        s("pyth", "Pyth Hermes 价格", "data", 103, 60, "https://hermes.pyth.network"),
        s("curve", "Curve API", "data", 104, 60, "https://api.curve.fi"),
        s("balancer", "Balancer API", "data", 105, 60, "https://api.balancer.fi"),
        s("coinpaprika", "CoinPaprika", "data", 106, 60, "https://api.coinpaprika.com/v1"),
        s("cryptocompare", "CryptoCompare", "data", 107, 60, "https://min-api.cryptocompare.com/data"),
        s("moralis", "Moralis", "data", 108, 60, "https://deep-index.moralis.io/api/v2.2"),
        s("covalent", "Covalent / GoldRush", "data", 109, 60, "https://api.covalenthq.com/v1"),
        s("quicknode_rpc", "QuickNode RPC（免费档）", "rpc", 110, 300, "https://your-endpoint.quiknode.pro"),
        s("ankr_rpc", "Ankr RPC", "rpc", 111, 300, "https://rpc.ankr.com/arbitrum"),
        s("sourcify", "Sourcify 源码验证", "explorer", 112, 30, "https://sourcify.dev/server"),
    ];
    // 免费优先：需要 Key 的源默认关闭，填好 Key 后可在页面手动启用。
    for src in &mut list {
        if KEY_REQUIRED.contains(&src.id.as_str()) {
            src.enabled = false;
        }
    }
    list
}

impl SourceConfig {
    fn path(workspace: &Path) -> PathBuf {
        workspace.join("data").join("source_config.json")
    }

    pub fn load(workspace: &Path) -> Self {
        let path = Self::path(workspace);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(mut cfg) = serde_json::from_str::<SourceConfig>(&text) {
                // 版本升级：补齐新增的默认数据源，保留已有配置与 Key。
                let mut changed = false;
                for d in default_sources() {
                    if !cfg.sources.iter().any(|s| s.id == d.id) {
                        cfg.sources.push(d);
                        changed = true;
                    }
                }
                // 免费优先：需要 Key 但没有 Key 的源强制关闭。
                for src in &mut cfg.sources {
                    if KEY_REQUIRED.contains(&src.id.as_str()) && src.api_key.trim().is_empty() {
                        if src.enabled {
                            src.enabled = false;
                            changed = true;
                        }
                    }
                }
                if changed {
                    cfg.save(workspace);
                }
                return cfg;
            }
        }
        let cfg = SourceConfig::default();
        cfg.save(workspace);
        cfg
    }

    /// 一键预设：free 仅启用免 Key 源；boost 启用免 Key + 已配置 Key 的源。
    pub fn apply_preset(&mut self, mode: &str) -> Vec<String> {
        let mut warnings = Vec::new();
        match mode {
            "free" => {
                self.strategy = "round_robin".into();
                self.interval_sec = 1_800;
                for s in &mut self.sources {
                    s.enabled = FREE_IDS.contains(&s.id.as_str());
                }
            }
            "boost" => {
                self.strategy = "priority".into();
                self.interval_sec = 900;
                for s in &mut self.sources {
                    let keyed = !s.api_key.trim().is_empty();
                    s.enabled = FREE_IDS.contains(&s.id.as_str()) || keyed;
                    if KEY_REQUIRED.contains(&s.id.as_str()) && !keyed {
                        warnings.push(format!("{} 未配置 Key，已保持关闭", s.name));
                    }
                }
            }
            _ => {
                warnings.push("未知预设，仅支持 free / boost".into());
            }
        }
        warnings
    }

    pub fn save(&self, workspace: &Path) {
        let path = Self::path(workspace);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// 参与候选发现的数据源。
    pub fn discovery_sources(&self) -> Vec<SourceEntry> {
        let mut list: Vec<SourceEntry> = self
            .sources
            .iter()
            .filter(|s| {
                s.enabled && matches!(s.kind.as_str(), "gecko" | "coingecko" | "dexscreener")
            })
            .cloned()
            .collect();
        if self.strategy == "priority" {
            list.sort_by_key(|s| s.priority);
        }
        list
    }

    pub fn key_for(&self, id: &str) -> String {
        self.sources
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.api_key.clone())
            .unwrap_or_default()
    }

    pub fn mark_used(&mut self, id: &str, status: &str) {
        if let Some(s) = self.sources.iter_mut().find(|s| s.id == id) {
            s.last_status = status.to_string();
            s.last_used_at = chrono::Utc::now().to_rfc3339();
        }
    }

    /// 返回给前端的脱敏副本。
    pub fn masked(&self) -> Self {
        let mut c = self.clone();
        for s in &mut c.sources {
            if !s.api_key.is_empty() {
                let keep = s.api_key.chars().take(4).collect::<String>();
                s.api_key = format!("{keep}****");
            }
        }
        c
    }

    /// 前端提交时：掩码/空值保留旧 Key。
    pub fn merge_keys(&mut self, incoming: &SourceConfig) {
        for s in &mut self.sources {
            if let Some(n) = incoming.sources.iter().find(|x| x.id == s.id) {
                if !n.api_key.is_empty() && !n.api_key.contains("****") {
                    s.api_key = n.api_key.clone();
                }
            }
        }
    }
}
