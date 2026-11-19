import type { ScanEvent } from "../components/ScanStrip";

export const scanDexEvents: ScanEvent[] = [
  { tone: "ok", text: "报价刷新 Base · WETH/USDC · 36 ms" },
  { tone: "info", text: "候选 21,776 → 剪枝 1,842 → 模拟 312" },
  { tone: "ok", text: "Arbitrum 区块 372,451,889 · 12 池状态更新" },
  { tone: "warn", text: "Sushi WETH/USDT 深度不足，跳过" },
  { tone: "ok", text: "Curve USDC/USDT 池费 0.01% 已校验" },
  { tone: "ok", text: "新机会 OPP-7F3A21 · 净利 +4.02" },
];

export const scanCexEvents: ScanEvent[] = [
  { tone: "ok", text: "ETH/USDT seq 8,821,451 · 缺口 0" },
  { tone: "ok", text: "盘口深度 84,200 USDT · 年龄 42 ms" },
  { tone: "warn", text: "WBTC/USDT 缺口 1 → 标记 INVALID" },
  { tone: "ok", text: "ARB/USDT 费率 8 bps 已同步" },
  { tone: "ok", text: "新机会 OPP-2B71FC · 净利 +5.50" },
  { tone: "info", text: "对冲完成率 25.0% · 敞口 3.40 USDC" },
];
