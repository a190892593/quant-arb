import type { IconName } from "../lib/icons";

export interface NavItem {
  id: string;
  label: string;
  icon: IconName;
  hash: string;
  badge?: () => number;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const NAV: NavGroup[] = [
  {
    label: "监控 MONITOR",
    items: [
      { id: "overview", label: "总览", icon: "overview", hash: "#/overview" },
    ],
  },
  {
    label: "搜索 SEARCH",
    items: [
      { id: "scannerDex", label: "DEX–DEX 搜索引擎", icon: "scanner", hash: "#/scanner-dex" },
      { id: "sources", label: "搜索数据源配置", icon: "system", hash: "#/sources" },
    ],
  },
  {
    label: "交易 PAPER",
    items: [
      { id: "executions", label: "Paper 执行", icon: "route", hash: "#/executions" },
      { id: "accounts", label: "账户与库存", icon: "wallet", hash: "#/accounts" },
      { id: "performance", label: "绩效", icon: "chart", hash: "#/performance" },
    ],
  },
  {
    label: "策略 STRATEGY",
    items: [
      { id: "strategy:dex_triple", label: "三角套利", icon: "triangle", hash: "#/strategy/dex_triple" },
      { id: "strategy:dex_cross", label: "跨池套利", icon: "cross", hash: "#/strategy/dex_cross" },
      { id: "strategy:cex_dex", label: "CEX–DEX 对冲", icon: "hedge", hash: "#/strategy/cex_dex" },
    ],
  },
  {
    label: "研究 RESEARCH",
    items: [
      { id: "pools", label: "池与 Token", icon: "pools", hash: "#/pools" },
      { id: "audit", label: "历史审计", icon: "audit", hash: "#/audit" },
      { id: "dataquality", label: "数据质量", icon: "data", hash: "#/dataquality" },
    ],
  },
  {
    label: "治理 GOVERN",
    items: [
      { id: "ledger", label: "账本", icon: "ledger", hash: "#/ledger" },
      { id: "risk", label: "风控台", icon: "shield", hash: "#/risk" },
      { id: "contract", label: "执行合约", icon: "contract", hash: "#/contract" },
      { id: "system", label: "系统", icon: "system", hash: "#/system" },
      { id: "capability", label: "能力矩阵", icon: "check", hash: "#/capability" },
    ],
  },
];
