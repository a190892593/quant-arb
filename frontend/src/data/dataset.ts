import { makeRng } from "../lib/rng";
import type {
  Adapter, ArchiveRow, AuditJob, BookIntegrity, ChainInfo, CircuitRule, ContractComponent, ContractEvent,
  ContractTest, DataModel, DexProtocol, EquityPoint, Exchange, Execution, FlashLoan, GapRow, Instrument,
  LedgerAccount, LedgerEntry, Opportunity, OpportunityCosts, Pool, RiskEvent, RiskParam, RpcNode, Service,
  SkewRow, StrategyConfig, TokenRow, Venue,
} from "../types";

type RawOpp = Omit<Opportunity, "costTotal" | "net" | "minProfit" | "netBps" | "theoBps" | "gasRatio">;

const rng = makeRng(20261119);

  /* ---------- reference prices ---------- */
  const px: Record<string, number> = { WETH: 2793.10, USDT: 1.0002, USDC: 1, ARB: 0.6120, WBTC: 68640.00 };

  /* ---------- venues & inventory ---------- */
  const venuesRaw: Omit<Venue, "value">[] = [
    {
      id: "dex-arb", kind: "DEX", name: "DEX 钱包 · Arbitrum", short: "DEX · ARB",
      chain: "Arbitrum", address: "0x7Ae1…4C9d", state: "PAPER_READY",
      assets: [{ sym: "USDC", qty: 1842.60 }, { sym: "WETH", qty: 0.4120 }]
    },
    {
      id: "dex-base", kind: "DEX", name: "DEX 钱包 · Base", short: "DEX · BASE",
      chain: "Base", address: "0x21Bc…88Aa", state: "PAPER_READY",
      assets: [{ sym: "USDC", qty: 320.40 }, { sym: "WETH", qty: 0.1135 }]
    },
    {
      id: "cex-bn", kind: "CEX", name: "CEX 现货 · Binance", short: "CEX · BINANCE",
      exchange: "Binance", state: "PAPER",
      assets: [{ sym: "USDT", qty: 1231.20 }, { sym: "ETH", qty: 0.1148 }]
    },
    {
      id: "flash", kind: "CLEARING", name: "闪电贷清算户", short: "FLASH CLEARING",
      chain: "Arbitrum", state: "IDLE", assets: [{ sym: "USDC", qty: 0 }]
    }
  ];

  function priceOf(sym: string): number {
    if (sym === "ETH") return px.WETH;
    return px[sym] ?? 0;
  }
  function venueValue(v: { assets: { sym: string; qty: number }[] }): number {
    return v.assets.reduce((s, a) => s + a.qty * priceOf(a.sym), 0);
  }
  const venues: Venue[] = venuesRaw.map((v) => ({ ...v, value: venueValue(v) }));

  const equity = venues.reduce((s, v) => s + v.value, 0);
  const startEquity = 5000;

  /* ---------- equity curve (90 sessions) ---------- */
  const days = 90;
  const todayPnlTarget = 11.86;
  const step = (equity - startEquity - todayPnlTarget) / (days - 2);
  const path = [startEquity];
  let e = startEquity;
  for (let i = 1; i < days - 1; i++) {
    const noise = (rng() - 0.5) * 8.6;
    const shock = i === 31 ? -14 : i === 41 ? -34 : i === 42 ? -14 : i === 43 ? -8 : i === 55 ? -10 : i === 63 ? 16 : i === 74 ? -18 : 0;
    e += step + noise + shock;
    path.push(e);
  }
  // remove drift so the walk lands exactly on the target before the last session
  const resid = path[days - 2] - (equity - todayPnlTarget);
  for (let i = 0; i < days - 1; i++) path[i] -= resid * (i / (days - 2));
  path.push(equity);

  const curve: EquityPoint[] = path.map((v, i) => ({
    i,
    equity: v,
    date: new Date(2026, 7, 22 + i), // 2026-08-22 + i days
    peak: v,
    dd: 0
  }));

  let peak = -Infinity;
  curve.forEach((p) => {
    peak = Math.max(peak, p.equity);
    p.peak = peak;
    p.dd = (peak - p.equity) / peak;
  });
  const maxDD = Math.max.apply(null, curve.map((p) => p.dd));
  const maxDDusd = Math.max.apply(null, curve.map((p) => (p.peak - p.equity)));
  const todayPnl = curve[curve.length - 1].equity - curve[curve.length - 2].equity;
  const weekPnl = curve[curve.length - 1].equity - curve[curve.length - 8].equity;

  /* ---------- opportunities ---------- */
  const C0 = (o?: Partial<OpportunityCosts>): OpportunityCosts => Object.assign({ dexFee: 0, cexFee: 0, gas: 0, priority: 0, flash: 0, slippage: 0, hedge: 0, rebalance: 0 }, o || {});

  const rawOpps: RawOpp[] = [
    {
      id: "OPP-7F3A21", strategy: "dex_triple", chain: "arbitrum",
      venue: "UNI V3 · CURVE · SUSHI", pair: "WETH → USDC → USDT → WETH",
      notional: 2500, theo: 11.47, gross: 9.86,
      costs: C0({ dexFee: 3.42, gas: 1.18, priority: 0.24, flash: 0.29, slippage: 0.71 }),
      stressNet: 0.18, riskScore: 78, quality: "A", status: "ACTIONABLE",
      ageMs: 312, model: "M2", block: 372451889, blockDelta: 0, quoteAgeMs: 312,
      facts: { hops: 3, impactBps: 10.4, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 71 },
      legs: [
        { n: 1, from: "WETH", to: "USDC", pool: "Uniswap V3", pair: "WETH/USDC", fee: "0.05%", inQty: "0.8951 WETH", outQty: "2,504.60 USDC", minOut: "2,499.59", impact: "4.1 bps" },
        { n: 2, from: "USDC", to: "USDT", pool: "Curve", pair: "3pool", fee: "0.01%", inQty: "2,504.60 USDC", outQty: "2,505.02 USDT", minOut: "2,502.51", impact: "0.8 bps" },
        { n: 3, from: "USDT", to: "WETH", pool: "SushiSwap V2", pair: "WETH/USDT", fee: "0.30%", inQty: "2,505.02 USDT", outQty: "0.8992 WETH", minOut: "0.8974", impact: "5.5 bps" }
      ]
    },
    {
      id: "OPP-91C4B0", strategy: "cex_dex", chain: "arbitrum", cex: "Binance", symbol: "ETH/USDT",
      venue: "BINANCE · UNI V3", pair: "DEX 买入 → CEX 卖出",
      notional: 1800, theo: 9.20, gross: 7.95,
      costs: C0({ dexFee: 0.95, cexFee: 1.44, gas: 1.02, priority: 0.21, slippage: 1.18, hedge: 0.90, rebalance: 0.35 }),
      stressNet: -2.10, riskScore: 42, quality: "A", status: "REJECTED", model: "M2",
      ageMs: 184, block: 372451887, blockDelta: 0, quoteAgeMs: 184,
      facts: { hops: 1, impactBps: 8.2, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 58, depthUsage: 13.2, bookAgeMs: 184 },
      reasons: ["最低保守净利润 5.40 USDC > 净利 1.90", "CEX 盘口深度占用 13.2% > 10%"]
    },
    {
      id: "OPP-2D8E57", strategy: "dex_cross", chain: "base",
      venue: "UNI V3 · AERODROME", pair: "WETH → USDC → WETH（跨池）",
      notional: 1200, theo: 6.10, gross: 5.42,
      costs: C0({ dexFee: 1.86, gas: 0.42, priority: 0.09, slippage: 0.38 }),
      stressNet: 0.94, riskScore: 61, quality: "A", status: "WATCH", model: "M2",
      ageMs: 96, block: 34882110, blockDelta: 0, quoteAgeMs: 96,
      facts: { hops: 2, impactBps: 11.8, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 42, depthUsage: 6.1 },
      reasons: ["竞争评分 42/100：近 10 次同类路线 4 次被抢先成交"]
    },
    {
      id: "OPP-C30A19", strategy: "dex_triple", chain: "ethereum",
      venue: "UNI V3 · CURVE · BALANCER", pair: "USDC → WETH → USDT → USDC",
      notional: 5000, theo: 14.80, gross: 11.20,
      costs: C0({ dexFee: 6.10, gas: 3.40, priority: 1.10, flash: 0.42, slippage: 1.05 }),
      stressNet: -3.40, riskScore: 34, quality: "A", status: "REJECTED", model: "M2",
      ageMs: 421, block: 21884102, blockDelta: 0, quoteAgeMs: 421,
      facts: { hops: 3, impactBps: 14.2, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 55 },
      reasons: ["Gas / 毛利 30.4% > 25% 硬上限", "最低净利润 5.00 USDC > 净利 −0.87"]
    },
    {
      id: "OPP-5B77E2", strategy: "cex_dex", chain: "base", cex: "Binance", symbol: "ETH/USDT",
      venue: "BINANCE · AERODROME", pair: "CEX 买入 → DEX 卖出",
      notional: 2500, theo: 21.40, gross: 18.20,
      costs: C0({ dexFee: 2.10, cexFee: 2.00, gas: 0.46, priority: 0.08, slippage: 2.42, hedge: 1.10, rebalance: 0.40 }),
      stressNet: 2.80, riskScore: 66, quality: "B", status: "WATCH", model: "M2",
      ageMs: 640, block: 34882108, blockDelta: 1, quoteAgeMs: 640,
      facts: { hops: 1, impactBps: 12.9, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 64, depthUsage: 7.4, bookAgeMs: 640 },
      reasons: ["盘口年龄 640 ms > 250 ms → 盘口不可用于生成可执行机会"]
    },
    {
      id: "OPP-A1F093", strategy: "dex_cross", chain: "arbitrum",
      venue: "SUSHI · UNI V3", pair: "USDC → WETH → USDC（跨池）",
      notional: 1500, theo: 4.80, gross: 4.10,
      costs: C0({ dexFee: 1.42, gas: 0.48, priority: 0.09, slippage: 0.31 }),
      stressNet: 0.32, riskScore: 58, quality: "A", status: "EXPIRED", model: "M2",
      ageMs: 3810, block: 372451841, blockDelta: 3, quoteAgeMs: 3810,
      facts: { hops: 2, impactBps: 7.7, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 60 },
      reasons: ["报价年龄 3.81 s 超过有效期 2.40 s"]
    },
    {
      id: "OPP-1180CC", strategy: "dex_triple", chain: "base",
      venue: "UNI V3 · AERODROME · CURVE", pair: "USDC → WETH → USDC → USDC",
      notional: 800, theo: 4.20, gross: 3.90,
      costs: C0({ dexFee: 0.98, gas: 0.36, priority: 0.08, slippage: 0.20 }),
      stressNet: 1.10, riskScore: 74, quality: "A", status: "EXECUTED", model: "M2",
      ageMs: 4200, block: 34882090, blockDelta: 6, quoteAgeMs: 4200,
      facts: { hops: 3, impactBps: 6.9, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 69 },
      executionId: "EX-2026-1119-0038"
    },
    {
      id: "OPP-6E42DA", strategy: "cex_dex", chain: "arbitrum", cex: "OKX", symbol: "ETH/USDT",
      venue: "OKX · UNI V3", pair: "DEX 卖出 → CEX 买入",
      notional: 3000, theo: 24.00, gross: 20.40,
      costs: C0({ dexFee: 1.60, cexFee: 2.40, gas: 0.94, priority: 0.18, slippage: 2.02, hedge: 1.06, rebalance: 0.40 }),
      stressNet: 5.20, riskScore: 55, quality: "A", status: "REJECTED", model: "M2",
      ageMs: 210, block: 372451880, blockDelta: 0, quoteAgeMs: 210,
      facts: { hops: 1, impactBps: 9.4, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: false, competition: 66, depthUsage: 4.8, bookAgeMs: 210 },
      reasons: ["CEX 库存 0.4200 ETH < 所需 0.9820 ETH", "OKX 适配器仅预留接口，未启用"]
    },
    {
      id: "OPP-3F90B1", strategy: "dex_cross", chain: "polygon",
      venue: "UNI V3 · QUICKSWAP", pair: "USDC → WETH → USDC（跨池）",
      notional: 1200, theo: 8.00, gross: 6.50,
      costs: C0({ dexFee: 2.02, gas: 0.18, priority: 0.04, slippage: 0.96 }),
      stressNet: 1.20, riskScore: 47, quality: "C", status: "REJECTED", model: "M1",
      ageMs: 730, block: 63120447, blockDelta: 2, quoteAgeMs: 730,
      facts: { hops: 2, impactBps: 18.8, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 38 },
      reasons: ["链状态 DATA_VALIDATED，未达 PAPER_READY", "主备 RPC 区块差 2 个区块"]
    },
    {
      id: "OPP-B8D5E6", strategy: "dex_triple", chain: "arbitrum",
      venue: "UNI V3 · CURVE · BALANCER", pair: "WETH → USDC → DAI → WETH",
      notional: 4000, theo: 18.00, gross: 13.20,
      costs: C0({ dexFee: 4.80, gas: 1.24, priority: 0.30, flash: 0.46, slippage: 1.96 }),
      stressNet: 2.10, riskScore: 63, quality: "A", status: "WATCH", model: "M2",
      ageMs: 168, block: 372451888, blockDelta: 0, quoteAgeMs: 168,
      facts: { hops: 3, impactBps: 24.6, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 72 },
      reasons: ["全路线价格冲击 24.6 bps > 20 bps 上限"]
    },
    {
      id: "OPP-9C2E4F", strategy: "cex_dex", chain: "arbitrum", cex: "Binance", symbol: "ETH/USDT",
      venue: "BINANCE · UNI V3", pair: "DEX 买入 → CEX 卖出",
      notional: 2200, theo: 17.60, gross: 15.10,
      costs: C0({ dexFee: 1.72, cexFee: 1.76, gas: 0.88, priority: 0.14, slippage: 1.94, hedge: 0.82, rebalance: 0.26 }),
      stressNet: 1.32, riskScore: 71, quality: "A", status: "ACTIONABLE", model: "M3",
      ageMs: 88, block: 372451889, blockDelta: 0, quoteAgeMs: 88,
      facts: { hops: 1, impactBps: 9.8, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 68, depthUsage: 6.2, bookAgeMs: 88 },
      legs: [
        { n: 1, from: "USDC", to: "WETH", pool: "Uniswap V3", pair: "WETH/USDC", fee: "0.05%", inQty: "787.76 USDC", outQty: "0.2820 WETH", minOut: "0.2813", impact: "9.8 bps" },
        { n: 2, from: "WETH", to: "USDT", pool: "Binance 现货", pair: "ETH/USDT", fee: "8 bps", inQty: "0.2820 ETH", outQty: "788.30 USDT", minOut: "787.20", impact: "IOC · 深度占用 6.2%" }
      ]
    },
    {
      id: "OPP-47AA02", strategy: "dex_cross", chain: "arbitrum",
      venue: "UNI V3 · SUSHI", pair: "USDC → WETH → USDC（跨池）",
      notional: 600, theo: 3.40, gross: 3.10,
      costs: C0({ dexFee: 0.46, gas: 0.34, priority: 0.08, slippage: 0.14 }),
      stressNet: 1.62, riskScore: 64, quality: "A", status: "ACTIONABLE", model: "M2",
      ageMs: 142, block: 372451889, blockDelta: 0, quoteAgeMs: 142,
      facts: { hops: 2, impactBps: 5.2, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 62 }
    },
    {
      id: "OPP-2B71FC", strategy: "cex_dex", chain: "arbitrum", cex: "Binance", symbol: "ARB/USDT",
      venue: "BINANCE · UNI V3", pair: "CEX 买入 → DEX 卖出",
      notional: 1200, theo: 9.40, gross: 8.60,
      costs: C0({ dexFee: 0.28, cexFee: 0.96, gas: 0.46, priority: 0.07, slippage: 0.82, hedge: 0.38, rebalance: 0.13 }),
      stressNet: 0.60, riskScore: 69, quality: "A", status: "ACTIONABLE", model: "M2",
      ageMs: 58, block: 372451888, blockDelta: 0, quoteAgeMs: 58,
      facts: { hops: 1, impactBps: 11.2, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 63, depthUsage: 7.1, bookAgeMs: 58 }
    },
    {
      id: "OPP-8D3E0A", strategy: "cex_dex", chain: "arbitrum", cex: "Binance", symbol: "WBTC/USDT",
      venue: "BINANCE · UNI V3", pair: "DEX 卖出 → CEX 买入",
      notional: 5000, theo: 30.20, gross: 27.40,
      costs: C0({ dexFee: 1.10, cexFee: 4.00, gas: 0.62, priority: 0.09, slippage: 3.10, hedge: 1.20, rebalance: 0.45 }),
      stressNet: 3.20, riskScore: 58, quality: "B", status: "WATCH", model: "M2",
      ageMs: 268, block: 372451886, blockDelta: 0, quoteAgeMs: 268,
      facts: { hops: 1, impactBps: 9.6, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 57, depthUsage: 8.4, bookAgeMs: 268 },
      reasons: ["盘口年龄 268 ms > 250 ms → 盘口不可用于生成可执行机会"]
    },
    {
      id: "OPP-4C9A17", strategy: "dex_cross", chain: "ethereum",
      venue: "UNI V3 · SUSHI", pair: "WBTC → USDC → WBTC",
      notional: 6000, theo: 12.40, gross: 9.80,
      costs: C0({ dexFee: 3.60, gas: 3.90, priority: 0.80, slippage: 1.20 }),
      stressNet: -4.20, riskScore: 41, quality: "A", status: "REJECTED", model: "M2",
      ageMs: 388, block: 21884102, blockDelta: 0, quoteAgeMs: 388,
      facts: { hops: 2, impactBps: 13.6, rpcOk: true, tokenOk: true, providerOk: true, inventoryOk: true, competition: 49 },
      reasons: ["Gas / 毛利 39.8% > 25% 硬上限", "最低净利润 6.00 USDC > 净利 0.30"]
    }
  ];

  const enriched: Opportunity[] = rawOpps.map((o) => {
    const costTotal = (Object.keys(o.costs) as (keyof OpportunityCosts)[]).reduce((s, k) => s + o.costs[k], 0);
    const net = o.gross - costTotal;
    const minProfit = Math.max(o.strategy === "cex_dex" ? 5 : 2, o.notional * (o.strategy === "cex_dex" ? 0.003 : 0.001));
    return { ...o, costTotal, net, minProfit, netBps: (net / o.notional) * 10000, theoBps: (o.theo / o.notional) * 10000, gasRatio: (o.costs.gas / o.gross) * 100 };
  });
  const opps = enriched;

  const STRATEGY = {
    dex_triple: { label: "DEX 三角", short: "三角", color: "accent" },
    dex_cross: { label: "DEX 跨池", short: "跨池", color: "accent" },
    cex_dex: { label: "CEX–DEX 对冲", short: "对冲", color: "warn" }
  };
  const CHAIN = {
    arbitrum: { label: "Arbitrum", id: 42161 },
    base: { label: "Base", id: 8453 },
    ethereum: { label: "Ethereum", id: 1 },
    polygon: { label: "Polygon", id: 137 }
  };

  /* ---------- executions ---------- */
  const executions: Execution[] = [
    {
      id: "EX-2026-1119-0042", oppId: "OPP-9C2E4F", strategy: "cex_dex", chain: "arbitrum",
      cex: "Binance", symbol: "ETH/USDT", pair: "DEX 买入 → CEX 卖出",
      notional: 2200, state: "HEDGED", order: "DEX 先行",
      startedAt: "2026-11-19T16:41:22+08:00", duration: 1840, exposure: 0, exposureMs: 980,
      netPnl: 7.58, statusNote: "两腿均已核对，敞口归零",
      legs: [
        { side: "DEX", venue: "Uniswap V3 · Arbitrum", state: "CONFIRMED", submitted: "16:41:22.140", filled: "16:41:23.980", qty: "0.2818 WETH", price: "2793.42", fee: "0.11 USDC", slip: "9.8 bps" },
        { side: "CEX", venue: "Binance 现货", state: "FILLED", submitted: "16:41:23.102", filled: "16:41:23.842", qty: "0.2818 ETH", price: "2795.18", fee: "1.76 USDT", slip: "2.1 bps" }
      ],
      ledger: [{ account: "DEX · ARB", delta: "−787.76 USDC" }, { account: "DEX · ARB", delta: "+0.2818 WETH" }, { account: "CEX · BINANCE", delta: "−0.2818 ETH" }, { account: "CEX · BINANCE", delta: "+788.30 USDT" }]
    },
    {
      id: "EX-2026-1119-0041", oppId: "OPP-B8D5E6", strategy: "dex_triple", chain: "arbitrum",
      pair: "WETH → USDC → DAI → WETH", notional: 4000, state: "DEX_REVERTED", order: "DEX 原子",
      startedAt: "2026-11-19T16:22:08+08:00", duration: 2120, exposure: 0, exposureMs: 0,
      netPnl: -1.24, statusNote: "第 3 腿最低输出未满足，整笔回滚；损失为 Gas，无残余敞口",
      legs: [
        { side: "DEX", venue: "ArbExecutor · 模拟", state: "REVERTED", submitted: "16:22:08.410", filled: "—", qty: "0", price: "—", fee: "1.24 USDC Gas", slip: "—" }
      ],
      revert: "SwapLegMinOutNotMet · leg 3 · expected ≥ 1.4182 WETH, received 1.4160 WETH"
    },
    {
      id: "EX-2026-1119-0040", oppId: "OPP-5B77E2", strategy: "cex_dex", chain: "base",
      cex: "Binance", symbol: "ETH/USDT", pair: "CEX 买入 → DEX 卖出",
      notional: 2500, state: "PARTIAL_HEDGE", order: "CEX 先行",
      startedAt: "2026-11-19T15:58:41+08:00", duration: 3240, exposure: 8.40, exposureMs: 2140,
      netPnl: 3.12, statusNote: "IOC 部分成交 62%，按实际成交量重算 DEX 腿，残余 8.40 USDC 已补偿",
      legs: [
        { side: "CEX", venue: "Binance 现货", state: "PARTIALLY_FILLED", submitted: "15:58:41.220", filled: "15:58:42.060", qty: "0.5540 / 0.8935 ETH", price: "2794.02", fee: "1.24 USDT", slip: "1.6 bps" },
        { side: "DEX", venue: "Aerodrome · Base", state: "FILLED", submitted: "15:58:42.480", filled: "15:58:44.360", qty: "0.5540 WETH", price: "2796.30", fee: "0.47 USDC", slip: "12.9 bps" }
      ],
      exposureSeries: [0, 3.1, 6.8, 8.4, 8.4, 5.2, 0]
    },
    {
      id: "EX-2026-1119-0039", oppId: "OPP-91C4B0", strategy: "cex_dex", chain: "arbitrum",
      cex: "Binance", symbol: "ETH/USDT", pair: "DEX 买入 → CEX 卖出",
      notional: 1800, state: "ORDER_STATUS_UNKNOWN", order: "CEX 先行",
      startedAt: "2026-11-19T15:12:03+08:00", duration: 9600, exposure: 0, exposureMs: 0,
      netPnl: 0, statusNote: "下单响应超时 → 标记状态未知 → 按 Client Order ID 查询为未成交 → 撤销，未重发",
      legs: [
        { side: "CEX", venue: "Binance 现货", state: "UNKNOWN", submitted: "15:12:03.118", filled: "—", qty: "查询确认 0 成交", price: "—", fee: "0", slip: "—" },
        { side: "DEX", venue: "Uniswap V3 · Arbitrum", state: "NOT_SUBMITTED", submitted: "—", filled: "—", qty: "—", price: "—", fee: "0", slip: "—" }
      ]
    },
    {
      id: "EX-2026-1119-0038", oppId: "OPP-1180CC", strategy: "dex_triple", chain: "base",
      pair: "USDC → WETH → USDC → USDC", notional: 800, state: "RECONCILED", order: "DEX 原子",
      startedAt: "2026-11-19T14:47:12+08:00", duration: 1680, exposure: 0, exposureMs: 0,
      netPnl: 2.28, statusNote: "自有资金闭环，账本与模拟余额一致",
      legs: [
        { side: "DEX", venue: "Uniswap V3 · Aerodrome · Curve", state: "CONFIRMED", submitted: "14:47:12.060", filled: "14:47:13.740", qty: "3 跳闭环", price: "—", fee: "0.98 USDC", slip: "6.9 bps" }
      ]
    },
    {
      id: "EX-2026-1119-0037", oppId: "OPP-6E42DA", strategy: "cex_dex", chain: "arbitrum",
      cex: "OKX", symbol: "ETH/USDT", pair: "DEX 卖出 → CEX 买入",
      notional: 3000, state: "REJECTED", order: "—",
      startedAt: "2026-11-19T14:20:55+08:00", duration: 12, exposure: 0, exposureMs: 0,
      netPnl: 0, statusNote: "Pre-Trade Risk Engine 拒绝：CEX 库存不足",
      legs: []
    },
    {
      id: "EX-2026-1119-0036", oppId: "OPP-5B77E2", strategy: "cex_dex", chain: "base",
      cex: "Binance", symbol: "ETH/USDT", pair: "DEX 先行 → CEX 对冲",
      notional: 2400, state: "UNWIND_REQUIRED", order: "DEX 先行",
      startedAt: "2026-11-19T13:38:20+08:00", duration: 4120, exposure: 12.60, exposureMs: 4120,
      netPnl: -5.80, statusNote: "DEX 腿已成交，CEX 两次 IOC 均未成交；残余 12.60 USDC 敞口转入补偿流程",
      legs: [
        { side: "DEX", venue: "Aerodrome · Base", state: "CONFIRMED", submitted: "13:38:20.140", filled: "13:38:21.980", qty: "0.8590 WETH", price: "2794.10", fee: "0.72 USDC", slip: "14.2 bps" },
        { side: "CEX", venue: "Binance 现货", state: "REJECTED", submitted: "13:38:22.120", filled: "—", qty: "0 / 0.8590 ETH", price: "—", fee: "0", slip: "—" }
      ],
      ledger: [{ account: "DEX · BASE", delta: "−2,400.00 USDC" }, { account: "DEX · BASE", delta: "+0.8590 WETH" }, { account: "HEDGE COMP", delta: "待补偿 12.60 敞口" }],
      exposureSeries: [0, 4.2, 8.8, 12.6, 12.6, 12.6, 12.6]
    },
    {
      id: "EX-2026-1119-0035", oppId: "OPP-2D8E57", strategy: "cex_dex", chain: "base",
      cex: "Binance", symbol: "ETH/USDT", pair: "CEX 先行 → DEX 对冲",
      notional: 3200, state: "MANUAL_RECOVERY_REQUIRED", order: "CEX 先行",
      startedAt: "2026-11-19T12:04:11+08:00", duration: 18600, exposure: 18.20, exposureMs: 18600,
      netPnl: -14.60, statusNote: "CEX 已成交，DEX 腿回滚；两次补偿报价均超出 10 bps 保护价，转入人工处置",
      legs: [
        { side: "CEX", venue: "Binance 现货", state: "FILLED", submitted: "12:04:11.220", filled: "12:04:12.010", qty: "1.1450 ETH", price: "2794.55", fee: "2.56 USDT", slip: "1.4 bps" },
        { side: "DEX", venue: "Uniswap V3 · Base", state: "REVERTED", submitted: "12:04:12.480", filled: "—", qty: "1.1450 / 1.1450", price: "—", fee: "0.44 USDC Gas", slip: "—" }
      ],
      revert: "SwapLegMinOutNotMet · leg 1 · expected ≥ 1.1420 WETH, received 1.1402 WETH",
      ledger: [{ account: "CEX · BINANCE", delta: "−3,200.00 USDT" }, { account: "CEX · BINANCE", delta: "+1.1450 ETH" }, { account: "DEX · BASE", delta: "待人工处置 18.20 敞口" }],
      exposureSeries: [0, 6.1, 11.4, 14.9, 18.2, 18.2]
    }
  ];

  const EXEC_STATES = ["DETECTED", "VALIDATED", "PLANNED", "SUBMITTED", "PARTIALLY_FILLED", "HEDGED", "RECONCILED"];

  /* ---------- ledger ---------- */
  const ledger: LedgerEntry[] = [
    { ts: "16:41:24", id: "LE-88216", account: "FLASH CLEARING", type: "FLASH_LOAN", memo: "Aave V3 偿还本金 + 费用 0.00045 WETH", amount: -0.89555, balance: 0, ref: "FL-2219", unit: "WETH" },
    { ts: "16:41:22", id: "LE-88215", account: "FLASH CLEARING", type: "FLASH_LOAN", memo: "Aave V3 借入 0.8951 WETH", amount: 0.8951, balance: 0.8951, ref: "FL-2219", unit: "WETH" },
    { ts: "16:41:23", id: "LE-88214", account: "CEX · BINANCE", type: "TRADE", memo: "对冲腿卖出 0.2818 ETH", amount: 788.30, balance: 1231.20, ref: "EX-0042" },
    { ts: "16:41:23", id: "LE-88213", account: "CEX · BINANCE", type: "TRADE", memo: "对冲腿卖出 0.2818 ETH（资产）", amount: -0.2818, balance: 0.1148, ref: "EX-0042", unit: "ETH" },
    { ts: "16:41:23", id: "LE-88212", account: "DEX · ARB", type: "FEE", memo: "Uniswap V3 池费 + Gas", amount: -0.99, balance: 1842.05, ref: "EX-0042" },
    { ts: "16:41:23", id: "LE-88211", account: "DEX · ARB", type: "TRADE", memo: "买入 0.2818 WETH", amount: 0.2818, balance: 0.4120, ref: "EX-0042", unit: "WETH" },
    { ts: "16:41:22", id: "LE-88210", account: "DEX · ARB", type: "TRADE", memo: "支付 787.76 USDC", amount: -787.76, balance: 1843.04, ref: "EX-0042" },
    { ts: "16:22:08", id: "LE-88209", account: "GAS EXPENSE", type: "GAS", memo: "回滚交易 Gas（DEX 失败仍计成本）", amount: -1.24, balance: -142.86, ref: "EX-0041" },
    { ts: "15:58:44", id: "LE-88208", account: "HEDGE COMP", type: "COMPENSATION", memo: "残余敞口补偿 8.40 USDC 的平仓损耗", amount: -0.34, balance: -58.12, ref: "EX-0040" },
    { ts: "15:58:42", id: "LE-88207", account: "CEX · BINANCE", type: "FEE", memo: "Binance 现货手续费 8 bps", amount: -1.24, balance: 442.90, ref: "EX-0040" },
    { ts: "15:12:03", id: "LE-88206", account: "CEX · BINANCE", type: "RESERVE", memo: "订单状态未知，冻结预留", amount: 0, balance: 444.14, ref: "EX-0039" },
    { ts: "14:47:13", id: "LE-88205", account: "REALIZED PNL", type: "PNL", memo: "三角闭环实现净利", amount: 2.28, balance: 182.63, ref: "EX-0038" },
    { ts: "14:47:13", id: "LE-88204", account: "DEX · BASE", type: "FEE", memo: "池费 + Gas", amount: -0.44, balance: 319.96, ref: "EX-0038" },
    { ts: "13:05:40", id: "LE-88203", account: "REBALANCE", type: "SETTLEMENT", memo: "库存再平衡：Base → Arbitrum 350 USDC", amount: -0.62, balance: -21.40, ref: "RB-2211" },
    { ts: "11:02:44", id: "LE-88202", account: "GAS EXPENSE", type: "GAS", memo: "扫描期间模拟 Gas 汇总", amount: -0.88, balance: -141.62, ref: "SYS" },
    { ts: "09:47:12", id: "LE-88201", account: "RISK RESERVE", type: "RESERVE", memo: "3 次/15 分钟回滚熔断，冻结相关策略额度", amount: 0, balance: 0, ref: "RE-3389" },
    { ts: "09:12:04", id: "LE-88200", account: "FUNDING", type: "SNAPSHOT", memo: "当日开盘权益快照", amount: 0, balance: 5170.77, ref: "SYS" },
    { ts: "08:55:19", id: "LE-88199", account: "INVENTORY", type: "SNAPSHOT", memo: "库存快照：DEX 3,630.6 / CEX 1,551.9", amount: 0, balance: 5182.44, ref: "SYS" }
  ];

  const LEDGER_TYPES = ["ALL", "TRADE", "FEE", "GAS", "COMPENSATION", "PNL", "RESERVE", "SETTLEMENT"];

  /* ---------- risk ---------- */
  const riskParamsRaw = [
    { key: "损失 · 全局当日亏损", limit: 50.00, cur: 12.40, unit: "USDC", pct: 24.8, severity: "HARD STOP", scope: "GLOBAL", v: "v1.0.3" },
    { key: "损失 · 单策略当日亏损", limit: 25.00, cur: 4.10, unit: "USDC", pct: 16.4, severity: "HARD STOP", scope: "dex_triple", v: "v1.0.3" },
    { key: "损失 · 最大权益回撤", limit: 250.00, cur: maxDDusd, unit: "USDC", pct: (maxDDusd / 250) * 100, severity: "HARD STOP", scope: "GLOBAL", v: "v1.0.3" },
    { key: "敞口 · 单 Token 合并", limit: 500.00, cur: 164.20, unit: "USDC", pct: 32.8, severity: "HARD STOP", scope: "WETH", v: "v1.0.3" },
    { key: "敞口 · 未对冲", limit: 25.00, cur: 3.40, unit: "USDC", pct: 13.6, severity: "HARD STOP", scope: "cex_dex", v: "v1.0.3" },
    { key: "时间 · 最大未对冲", limit: 2000, cur: 842, unit: "ms", pct: 42.1, severity: "HARD STOP", scope: "cex_dex", v: "v1.0.4" },
    { key: "数据 · 盘口最大年龄", limit: 250, cur: 96, unit: "ms", pct: 38.4, severity: "HARD STOP", scope: "Binance", v: "v1.0.4" },
    { key: "资源 · 单一场所资金使用率", limit: 80, cur: 62, unit: "%", pct: 77.5, severity: "SOFT BLOCK", scope: "EXCHANGE", v: "v1.0.3" },
    { key: "容量 · 闪电贷单笔借款", limit: 5000, cur: 2500, unit: "USDC", pct: 50.0, severity: "HARD STOP", scope: "Aave V3", v: "v1.0.2" },
    { key: "成本 · Gas / 毛利", limit: 25, cur: 12.0, unit: "%", pct: 48.0, severity: "HARD STOP", scope: "dex_triple", v: "v1.0.2" }
  ];

  const riskEvents: RiskEvent[] = [
    { id: "RE-3391", ts: "12:41:08", level: "SOFT_BLOCK", scope: "cex_dex", msg: "OPP-91C4B0 最低保守净利润 5.40 > 1.90，机会拒绝", state: "OPEN" },
    { id: "RE-3390", ts: "11:02:44", level: "WARNING", scope: "Base", msg: "RPC P95 延迟 612 ms 超预警线 500 ms，扫描降级", state: "OPEN" },
    { id: "RE-3389", ts: "09:47:12", level: "HARD_STOP", scope: "dex_triple · Arbitrum", msg: "3 次 / 15 分钟 DEX 回滚，暂停该策略 30 分钟", state: "RESOLVED" },
    { id: "RE-3388", ts: "昨日 22:15", level: "HARD_STOP", scope: "Binance · ETH/USDT", msg: "订单状态未知，禁止重发，已按 COID 查询为未成交", state: "RESOLVED" },
    { id: "RE-3387", ts: "昨日 20:03", level: "HARD_STOP", scope: "cex_dex", msg: "库存账本不一致 0.42 USDC，策略暂停并完成对账", state: "RESOLVED" }
  ];

  const scopes = [
    { group: "全局", items: [{ id: "global", label: "全局执行", on: true }] },
    { group: "策略", items: [
      { id: "s-triple", label: "DEX 三角", on: true },
      { id: "s-cross", label: "DEX 跨池", on: true },
      { id: "s-hedge", label: "CEX–DEX 对冲", on: true }
    ] },
    { group: "链", items: [
      { id: "c-arb", label: "Arbitrum", on: true },
      { id: "c-base", label: "Base", on: true },
      { id: "c-eth", label: "Ethereum", on: true },
      { id: "c-pol", label: "Polygon", on: false, note: "未达 PAPER_READY" }
    ] },
    { group: "交易所 / 交易对", items: [
      { id: "x-bn", label: "Binance", on: true },
      { id: "x-okx", label: "OKX", on: false, note: "仅接口预留" },
      { id: "p-ethusdt", label: "ETH/USDT", on: true },
      { id: "p-wbtc", label: "WBTC/USDT", on: true }
    ] },
    { group: "Token", items: [
      { id: "t-weth", label: "WETH", on: true },
      { id: "t-usdc", label: "USDC", on: true },
      { id: "t-usdt", label: "USDT", on: true },
      { id: "t-frax", label: "FRAX", on: false, note: "Fee-on-Transfer" }
    ] }
  ];

  /* ---------- audit ---------- */
  const auditJobs: AuditJob[] = [
    { id: "AJ-2026-1114-01", range: "2026-10-01 → 10-31", market: "Ethereum · Uniswap V3 / Curve", model: "M3", coverage: 99.2, gaps: 3, samples: 48210, verdict: "WATCH", note: "2 个区块区间归档状态不可读，已从审计样本剔除" },
    { id: "AJ-2026-1113-02", range: "2026-09-01 → 10-31", market: "Arbitrum · Uniswap V3 / Sushi", model: "M3", coverage: 100, gaps: 0, samples: 128640, verdict: "PASS", note: "含 L2 深度与逐笔成交，样本外区间通过" },
    { id: "AJ-2026-1112-03", range: "2026-10-15 → 11-14", market: "Base · Aerodrome / Uniswap V3", model: "M2", coverage: 97.8, gaps: 1, samples: 31402, verdict: "PASS", note: "缺少 1 段 WebSocket 断线区间" },
    { id: "AJ-2026-1110-04", range: "2026-10-01 → 11-14", market: "Binance · ETH/USDT L2", model: "M3", coverage: 86.4, gaps: 12, samples: 906114, verdict: "WATCH", note: "历史 L2 深度覆盖不足，仅可用于初筛" },
    { id: "AJ-2026-1108-05", range: "2026-09-15 → 10-31", market: "CEX–DEX 联合 · ETH", model: "M2", coverage: 94.1, gaps: 6, samples: 21840, verdict: "REJECT", note: "净利集中于 3 个交易日，剔除后区间净利转负" },
    { id: "AJ-2026-1106-06", range: "2026-11-01 → 11-14", market: "Polygon · Uniswap V3", model: "M1", coverage: 71.2, gaps: 9, samples: 6120, verdict: "PAPER", note: "数据区间不足 14 天，仅保留模拟观察" }
  ];

  /* ---------- system ---------- */
  const services: Service[] = [
    { name: "arb-api", role: "Axum API + WS Gateway", state: "UP", uptime: "8h 14m", p95: "18 ms", note: "127.0.0.1:8080" },
    { name: "arb-worker", role: "行情采集 / 池状态 / 扫描", state: "UP", uptime: "8h 14m", p95: "42 ms", note: "tokio · 6 tasks" },
    { name: "arb-paper", role: "Paper 撮合与账本结算", state: "UP", uptime: "8h 14m", p95: "9 ms", note: "账本差异 0.00" },
    { name: "arb-audit", role: "历史回放 / 审计报告", state: "IDLE", uptime: "—", p95: "—", note: "上一任务 11-14 完成" },
    { name: "postgres", role: "本机 PostgreSQL", state: "UP", uptime: "8h 14m", p95: "3 ms", note: "db: arbx_local" }
  ];

  const chainsBase = [
    { name: "Arbitrum", id: 42161, status: "PAPER_READY", stage: 4, rpc: "188 ms", rpc2: "204 ms", block: 372451889, lag: 0, gas: "0.012 gwei", pools: 326, quality: "A" },
    { name: "Base", id: 8453, status: "PAPER_READY", stage: 4, rpc: "96 ms", rpc2: "148 ms", block: 34882110, lag: 0, gas: "0.008 gwei", pools: 218, quality: "A" },
    { name: "Ethereum", id: 1, status: "HISTORICAL_READY", stage: 3, rpc: "214 ms", rpc2: "402 ms", block: 21884102, lag: 0, gas: "6.2 gwei", pools: 412, quality: "A" },
    { name: "Polygon", id: 137, status: "DATA_VALIDATED", stage: 2, rpc: "344 ms", rpc2: "498 ms", block: 63120447, lag: 2, gas: "32 gwei", pools: 96, quality: "C" }
  ];
  const CHAIN_STAGES = ["DISCOVERED", "DATA_VALIDATED", "HISTORICAL_READY", "PAPER_READY", "LIVE_CANDIDATE", "LIVE_APPROVED"];

  const dexProtocols: DexProtocol[] = [
    { name: "Uniswap V3", type: "CLMM", pools: 412, chains: 4, state: "ENABLED", adapter: "v3.1.2" },
    { name: "Uniswap V2", type: "CPMM", pools: 86, chains: 3, state: "ENABLED", adapter: "v2.0.6" },
    { name: "SushiSwap", type: "CPMM", pools: 54, chains: 2, state: "ENABLED", adapter: "v2.0.6" },
    { name: "Curve", type: "StableSwap", pools: 31, chains: 2, state: "ENABLED", adapter: "ss.1.0.3" },
    { name: "Balancer", type: "Weighted", pools: 22, chains: 1, state: "ENABLED", adapter: "wp.1.0.1" },
    { name: "Aerodrome", type: "CPMM", pools: 74, chains: 1, state: "ENABLED", adapter: "v1.4.0" },
    { name: "Uniswap V4", type: "Hook", pools: 0, chains: 0, state: "BLOCKED", adapter: "—" }
  ];

  const exchanges: Exchange[] = [
    { name: "Binance", state: "ENABLED", adapter: "2.4.1", symbols: 3, bookAge: "42 ms", latency: "38 ms", gaps: 0, feeSync: "16:40:02" },
    { name: "OKX", state: "INTERFACE_ONLY", adapter: "—", symbols: 0, bookAge: "—", latency: "—", gaps: 0, feeSync: "—" },
    { name: "Bybit", state: "PLANNED", adapter: "—", symbols: 0, bookAge: "—", latency: "—", gaps: 0, feeSync: "—" }
  ];

  /* ---------- CEX instrument universe (PRD §5.3 / TDD §5.1) ----------
     All subscribed instruments are streamed; only the ones with a verified
     DEX counterpart and complete trading rules are executable. */
  const instruments: Instrument[] = [
    { symbol: "ETH/USDT", base: "ETH", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: true,
      bid: 2792.84, ask: 2793.16, spread: 1.1, depth10: 84200, age: 42, integrity: "VALID", seq: 8821451,
      tick: 0.01, step: 0.0001, minNotional: 5.00, maker: 0.02, taker: 0.08, pricePrec: 2, qtyPrec: 4, depthScale: 1, recvTs: "16:49:31.920" },
    { symbol: "ARB/USDT", base: "ARB", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: true,
      bid: 0.6119, ask: 0.6122, spread: 4.9, depth10: 12800, age: 58, integrity: "VALID", seq: 8820004,
      tick: 0.0001, step: 0.1, minNotional: 5.00, maker: 0.02, taker: 0.08, pricePrec: 4, qtyPrec: 1, depthScale: 0.42, recvTs: "16:49:31.904" },
    { symbol: "WBTC/USDT", base: "WBTC", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: true,
      bid: 68638.10, ask: 68643.80, spread: 0.8, depth10: 21400, age: 268, integrity: "INVALID", seq: 2410882,
      tick: 0.1, step: 0.0001, minNotional: 10.00, maker: 0.02, taker: 0.08, pricePrec: 2, qtyPrec: 4, depthScale: 0.006, recvTs: "16:40:14.010" },
    { symbol: "SOL/USDT", base: "SOL", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: false,
      bid: 214.62, ask: 214.68, spread: 2.8, depth10: 46200, age: 46, integrity: "VALID", seq: 7712340,
      tick: 0.01, step: 0.001, minNotional: 5.00, maker: 0.02, taker: 0.08, pricePrec: 2, qtyPrec: 3, depthScale: 0.18, recvTs: "16:49:31.916" },
    { symbol: "LINK/USDT", base: "LINK", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: false,
      bid: 18.412, ask: 18.418, spread: 3.3, depth10: 18400, age: 52, integrity: "VALID", seq: 6610908,
      tick: 0.001, step: 0.01, minNotional: 5.00, maker: 0.02, taker: 0.08, pricePrec: 3, qtyPrec: 2, depthScale: 0.09, recvTs: "16:49:31.910" },
    { symbol: "OP/USDT", base: "OP", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: false,
      bid: 1.8421, ask: 1.8426, spread: 2.7, depth10: 9600, age: 64, integrity: "VALID", seq: 5580117,
      tick: 0.0001, step: 0.1, minNotional: 5.00, maker: 0.02, taker: 0.08, pricePrec: 4, qtyPrec: 1, depthScale: 0.28, recvTs: "16:49:31.898" },
    { symbol: "DAI/USDT", base: "DAI", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: false,
      bid: 0.9998, ask: 1.0001, spread: 3.0, depth10: 62000, age: 38, integrity: "VALID", seq: 4490221,
      tick: 0.0001, step: 1, minNotional: 5.00, maker: 0.01, taker: 0.06, pricePrec: 4, qtyPrec: 0, depthScale: 1.6, recvTs: "16:49:31.924" },
    { symbol: "POL/USDT", base: "POL", quote: "USDT", exchange: "Binance", state: "TRADING", tradable: false,
      bid: 0.4821, ask: 0.4824, spread: 6.2, depth10: 14200, age: 340, integrity: "INVALID", seq: 3902775,
      tick: 0.0001, step: 1, minNotional: 5.00, maker: 0.02, taker: 0.08, pricePrec: 4, qtyPrec: 0, depthScale: 0.9, recvTs: "16:49:29.660" }
  ];

  /* per-instrument depth ladders (snapshot + increments validated per TDD §5.2) */
  const QTY_PATTERN = [8.42, 12.10, 5.60, 18.30, 22.10, 9.40, 14.20, 31.00];
  function makeBook(ins: Instrument) {
    const gap = Math.max(ins.ask - ins.bid, ins.tick);
    const asks = [];
    const bids = [];
    for (let i = 0; i < 8; i++) {
      asks.push({ p: +(ins.ask + gap * i).toFixed(ins.pricePrec), q: +(QTY_PATTERN[i] * ins.depthScale).toFixed(2) });
      bids.push({ p: +(ins.bid - gap * i).toFixed(ins.pricePrec), q: +(QTY_PATTERN[7 - i] * ins.depthScale).toFixed(2) });
    }
    return { symbol: ins.symbol, exchange: ins.exchange, seq: ins.seq, age: ins.age, recvTs: ins.recvTs, asks, bids };
  }
  const orderbooks: Record<string, { symbol: string; exchange: string; seq: number; age: number; recvTs: string; asks: { p: number; q: number }[]; bids: { p: number; q: number }[] }> = {};
  instruments.forEach((ins) => { orderbooks[ins.symbol] = makeBook(ins); });

  const health = [
    { key: "RPC", value: "UP", note: "双节点哈希一致 · P95 188 ms" },
    { key: "CEX WS", value: "UP", note: "Binance · 重连 1 次 / 8h" },
    { key: "数据库", value: "UP", note: "PostgreSQL · 账本差异 0.00" },
    { key: "历史数据", value: "UP", note: "Parquet 分区 · 4 链" }
  ];

  /* ---------- chain registry detail (TDD §3.1) ---------- */
  const CHAIN_META: Record<string, { native: string; blockTime: string; finality: string; gasModel: string; adapter: string; enabledAt: string; ws: string; archive: boolean }> = {
    Arbitrum: { native: "ETH", blockTime: "0.25 s", finality: "Soft · L1 batch", gasModel: "EIP-1559 · L2", adapter: "arb.1.2.0", enabledAt: "2026-09-01", ws: "wss://arb1.ws", archive: true },
    Base: { native: "ETH", blockTime: "2.0 s", finality: "Soft · L1 batch", gasModel: "EIP-1559 · L2", adapter: "base.1.1.4", enabledAt: "2026-09-12", ws: "wss://base.ws", archive: true },
    Ethereum: { native: "ETH", blockTime: "12.0 s", finality: "~13 min · finalized", gasModel: "EIP-1559 · L1", adapter: "eth.1.4.2", enabledAt: "2026-08-22", ws: "wss://eth.ws", archive: true },
    Polygon: { native: "POL", blockTime: "2.1 s", finality: "~30 min · checkpoint", gasModel: "EIP-1559 · PoS", adapter: "pol.0.9.6", enabledAt: "2026-11-04", ws: "wss://polygon.ws", archive: false }
  };
  const chains: ChainInfo[] = chainsBase.map((c) => ({ ...c, ...CHAIN_META[c.name] }));

  /* ---------- DEX pools & tokens (PRD §5.2) ---------- */
  const pools: Pool[] = [
    { addr: "0xC31E…8f21", protocol: "Uniswap V3", pair: "WETH/USDC", fee: "0.05%", chain: "Arbitrum", tvl: 18420000, vol24: 6210000, state: "ENABLED", adapter: "v3.1.2", age: "42 ms" },
    { addr: "0x6F2A…11d0", protocol: "Uniswap V3", pair: "WETH/USDT", fee: "0.05%", chain: "Arbitrum", tvl: 9180000, vol24: 2840000, state: "ENABLED", adapter: "v3.1.2", age: "44 ms" },
    { addr: "0x9Be7…22c8", protocol: "Curve", pair: "USDC/USDT", fee: "0.01%", chain: "Arbitrum", tvl: 22600000, vol24: 9130000, state: "ENABLED", adapter: "ss.1.0.3", age: "38 ms" },
    { addr: "0x4D1c…9a10", protocol: "SushiSwap", pair: "WETH/USDT", fee: "0.30%", chain: "Arbitrum", tvl: 2140000, vol24: 486000, state: "ENABLED", adapter: "v2.0.6", age: "51 ms" },
    { addr: "0x8a71…0f3b", protocol: "Uniswap V3", pair: "WETH/USDC", fee: "0.05%", chain: "Base", tvl: 4260000, vol24: 1280000, state: "ENABLED", adapter: "v3.1.2", age: "36 ms" },
    { addr: "0xC0D3…7e42", protocol: "Aerodrome", pair: "WETH/USDC", fee: "0.30%", chain: "Base", tvl: 1680000, vol24: 512000, state: "ENABLED", adapter: "v1.4.0", age: "39 ms" },
    { addr: "0x11B8…4c05", protocol: "Uniswap V3", pair: "WETH/USDC", fee: "0.30%", chain: "Ethereum", tvl: 41200000, vol24: 8460000, state: "ENABLED", adapter: "v3.1.2", age: "214 ms" },
    { addr: "0x5E3a…bb90", protocol: "Balancer", pair: "WETH/USDC", fee: "0.05%", chain: "Ethereum", tvl: 6140000, vol24: 720000, state: "OBSERVE", adapter: "wp.1.0.1", age: "221 ms" },
    { addr: "0xE402…8d1c", protocol: "Uniswap V4", pair: "WETH/USDC", fee: "Hook", chain: "Base", tvl: 0, vol24: 0, state: "BLOCKED", adapter: "—", age: "—" },
    { addr: "0x71Fa…0c88", protocol: "QuickSwap", pair: "WETH/USDC", fee: "0.30%", chain: "Polygon", tvl: 940000, vol24: 128000, state: "OBSERVE", adapter: "v2.0.6", age: "344 ms" }
  ];

  const tokens: TokenRow[] = [
    { sym: "WETH", chain: "Arbitrum", addr: "0x82aF…DA21", dec: 18, hash: "0x4f1c…9e02", tags: ["标准 ERC-20"], start: true, mid: true, flash: true, state: "ALLOWED" },
    { sym: "USDC", chain: "Arbitrum", addr: "0xaf88…5831", dec: 6, hash: "0x7b3d…22c1", tags: ["标准 ERC-20"], start: true, mid: true, flash: true, state: "ALLOWED" },
    { sym: "USDT", chain: "Arbitrum", addr: "0xFd08…3a5d", dec: 6, hash: "0x2c8a…71fe", tags: ["标准 ERC-20", "可冻结"], start: false, mid: true, flash: true, state: "ALLOWED" },
    { sym: "DAI", chain: "Arbitrum", addr: "0xDA10…1aF3", dec: 18, hash: "0x9e05…4b7a", tags: ["标准 ERC-20"], start: false, mid: true, flash: false, state: "ALLOWED" },
    { sym: "WBTC", chain: "Arbitrum", addr: "0x2f2a…E0F8", dec: 8, hash: "0x5a71…c3d9", tags: ["标准 ERC-20", "代理合约"], start: true, mid: false, flash: false, state: "ALLOWED" },
    { sym: "ARB", chain: "Arbitrum", addr: "0x912C…cC85", dec: 18, hash: "0x1f92…aa30", tags: ["标准 ERC-20"], start: false, mid: true, flash: false, state: "ALLOWED" },
    { sym: "USDC.e", chain: "Arbitrum", addr: "0xFF97…61e7", dec: 6, hash: "0x8d40…15b2", tags: ["桥接资产", "待验证"], start: false, mid: false, flash: false, state: "OBSERVE" },
    { sym: "FRAX", chain: "Arbitrum", addr: "0x17FC…3b23", dec: 18, hash: "0x3c17…90de", tags: ["非标准", "Fee-on-Transfer"], start: false, mid: false, flash: false, state: "BLOCKED" }
  ];

  const adapters: Adapter[] = [
    { protocol: "Uniswap V3", chain: "Arbitrum", version: "v3.1.2", factory: "0x1F98…ad61", router: "0xE592…F7d1", hash: "0x71c0…2e48", audit: "2026-10-18", state: "ENABLED" },
    { protocol: "Uniswap V2", chain: "Arbitrum", version: "v2.0.6", factory: "0xc35D…b1af", router: "0x4752…1B79", hash: "0x2b91…7cc0", audit: "2026-09-30", state: "ENABLED" },
    { protocol: "SushiSwap", chain: "Arbitrum", version: "v2.0.6", factory: "0xc35D…b1af", router: "0x1b02…a0a1", hash: "0x2b91…7cc0", audit: "2026-09-30", state: "ENABLED" },
    { protocol: "Curve", chain: "Arbitrum", version: "ss.1.0.3", factory: "0x9AF1…4f05", router: "0x4b57…5B4A", hash: "0x5e74…0b13", audit: "2026-10-02", state: "ENABLED" },
    { protocol: "Balancer", chain: "Ethereum", version: "wp.1.0.1", factory: "0x8E9A…c2d1", router: "0xBA12…55ad", hash: "0x9a2f…6d70", audit: "2026-09-21", state: "ENABLED" },
    { protocol: "Aerodrome", chain: "Base", version: "v1.4.0", factory: "0x420D…1d05", router: "0xcF77…0f2A", hash: "0x6d18…3ba9", audit: "2026-10-11", state: "ENABLED" },
    { protocol: "Uniswap V4", chain: "Base", version: "—", factory: "—", router: "—", hash: "—", audit: "未审计", state: "BLOCKED" }
  ];

  /* ---------- CEX order book depth (TDD §5.2) ---------- */
  const orderbook = {
    symbol: "ETH/USDT", exchange: "Binance", seq: 8821451, exchangeTs: "16:49:31.882", recvTs: "16:49:31.920", age: 38,
    asks: [
      { p: 2793.16, q: 8.42 }, { p: 2793.22, q: 12.10 }, { p: 2793.30, q: 5.60 }, { p: 2793.44, q: 18.30 },
      { p: 2793.60, q: 22.10 }, { p: 2793.82, q: 9.40 }, { p: 2794.10, q: 14.20 }, { p: 2794.55, q: 31.00 }
    ],
    bids: [
      { p: 2792.84, q: 6.10 }, { p: 2792.70, q: 11.40 }, { p: 2792.55, q: 19.80 }, { p: 2792.40, q: 8.20 },
      { p: 2792.12, q: 26.40 }, { p: 2791.88, q: 12.70 }, { p: 2791.60, q: 17.90 }, { p: 2791.20, q: 40.20 }
    ]
  };
  // the executable pair keeps the hand-built book used by the opportunity detail
  orderbooks["ETH/USDT"] = orderbook;

  /* ---------- data quality (TDD §3.3) ---------- */
  const rpcNodes: RpcNode[] = [
    { chain: "Arbitrum", role: "PRIMARY", endpoint: "arb1.chainstack.io", block: 372451889, hash: "0x8f2c", p95: 188, err: 0.02, state: "UP" },
    { chain: "Arbitrum", role: "SECONDARY", endpoint: "arbitrum.public-rpc", block: 372451889, hash: "0x8f2c", p95: 204, err: 0.04, state: "UP" },
    { chain: "Base", role: "PRIMARY", endpoint: "base-mainnet.g.alchemy", block: 34882110, hash: "0x3a91", p95: 96, err: 0.01, state: "UP" },
    { chain: "Base", role: "SECONDARY", endpoint: "base.llamarpc", block: 34882110, hash: "0x3a91", p95: 148, err: 0.06, state: "UP" },
    { chain: "Ethereum", role: "PRIMARY", endpoint: "eth.quiknode.pro", block: 21884102, hash: "0xc410", p95: 214, err: 0.03, state: "UP" },
    { chain: "Ethereum", role: "SECONDARY", endpoint: "ethereum.public-rpc", block: 21884102, hash: "0xc410", p95: 402, err: 0.11, state: "UP" },
    { chain: "Polygon", role: "PRIMARY", endpoint: "polygon-mainnet.g.alchemy", block: 63120447, hash: "0x91ba", p95: 344, err: 0.28, state: "UP" },
    { chain: "Polygon", role: "SECONDARY", endpoint: "polygon-rpc.com", block: 63120445, hash: "0x77c1", p95: 498, err: 0.44, state: "DIVERGED" }
  ];

  const cexIntegrity: BookIntegrity[] = [
    { exchange: "Binance", symbol: "ETH/USDT", seq: 8821451, gaps: 0, reconnect: 1, age: 42, latency: 38, lastSync: "16:41:02", state: "VALID" },
    { exchange: "Binance", symbol: "ARB/USDT", seq: 8820004, gaps: 0, reconnect: 1, age: 58, latency: 41, lastSync: "16:41:02", state: "VALID" },
    { exchange: "Binance", symbol: "WBTC/USDT", seq: 2410882, gaps: 1, reconnect: 3, age: 268, latency: 96, lastSync: "16:40:14", state: "INVALID" }
  ];

  const archiveCoverage: ArchiveRow[] = [
    { chain: "Arbitrum", from: "2026-09-01", to: "2026-11-19", coverage: 100, readable: true, note: "全区间归档可读" },
    { chain: "Base", from: "2026-10-15", to: "2026-11-19", coverage: 97.8, readable: true, note: "1 段 WS 断线缺口" },
    { chain: "Ethereum", from: "2026-10-01", to: "2026-10-31", coverage: 99.2, readable: false, note: "2 个区块区间归档不可读" },
    { chain: "Polygon", from: "2026-11-01", to: "2026-11-14", coverage: 71.2, readable: true, note: "不足 14 天，仅可初筛" }
  ];

  const clockSkew: SkewRow[] = [
    { pair: "本地 ↔ Binance", skew: 38, limit: 250 },
    { pair: "本地 ↔ 交易所 P95", skew: 96, limit: 250 },
    { pair: "浏览器 ↔ 系统时钟", skew: 2, limit: 100 }
  ];

  const dataGaps: GapRow[] = [
    { at: "16:40:14", dur: "2.1 s", source: "Binance WS · WBTC/USDT", impact: "盘口标记 INVALID，等待重同步" },
    { at: "11:02:44", dur: "42 s", source: "Base RPC", impact: "该链扫描降级，未生成可执行机会" },
    { at: "09:47:12", dur: "8 s", source: "Arbitrum WS", impact: "重新拉取 Snapshot 后恢复" }
  ];

  /* ---------- execution contract (合约 §2 / §13) ---------- */
  const contractComponents: ContractComponent[] = [
    { name: "ArbExecutor", role: "原子执行入口与路线控制", addr: "0x9A3f…21c4", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "未部署主网" },
    { name: "DexAdapterRegistry", role: "已验证 DEX Adapter 登记", addr: "0x41bB…90aa", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "6 在册 / 1 阻断" },
    { name: "TokenRegistry", role: "Token 白名单与风险标签", addr: "0x77C0…4d19", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "6 允许 / 1 观察 / 1 阻断" },
    { name: "FundingProviderRegistry", role: "闪电贷提供方登记", addr: "0x2eDa…8b61", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "Aave V3 + ERC-3156" },
    { name: "AaveV3ReceiverAdapter", role: "Aave V3 回调适配", addr: "0x5C12…aa03", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "单资产 · 精确授权" },
    { name: "ERC3156ReceiverAdapter", role: "ERC-3156 回调适配", addr: "0x9b07…1f42", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "Magic Value 校验" },
    { name: "Treasury", role: "资金归集与受限提款", addr: "0x0Af2…7c55", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "仅非执行状态" },
    { name: "ExecutionPlanVerifier", role: "签名 / Nonce / 期限校验", addr: "0x63Dd…e910", version: "1.0.0-rc3", state: "LOCAL_ONLY", note: "EIP-712 domain" }
  ];

  const contractTests: ContractTest[] = [
    { group: "正常路径", total: 5, pass: 5, note: "自有资金闭环 / 闪电贷闭环 / 最低利润" },
    { group: "参数攻击", total: 12, pass: 12, note: "ChainID / 过期 / Nonce / 签名 / Token / Pool" },
    { group: "回调攻击", total: 7, pass: 7, note: "伪造 Provider / Initiator / 重复回调" },
    { group: "资金安全", total: 8, pass: 6, note: "偿还不足 / 最低利润 / 执行中提款" },
    { group: "DEX 风险", total: 7, pass: 7, note: "池地址伪造 / Factory 校验 / Hook 行为" }
  ];

  const contractEvents: ContractEvent[] = [
    { name: "PlanAccepted", n: 214, note: "计划通过校验" },
    { name: "ExecutionStarted", n: 214, note: "开始执行" },
    { name: "FlashLoanRequested", n: 96, note: "Aave V3 / ERC-3156" },
    { name: "SwapLegExecuted", n: 512, note: "单腿成交" },
    { name: "ProfitCheckPassed", n: 208, note: "最低利润满足" },
    { name: "ExecutionReverted", n: 6, note: "整笔回滚，损失 Gas" },
    { name: "PlanRejected", n: 0, note: "链上拒绝（本地模拟为 0）" },
    { name: "TreasuryWithdrawal", n: 0, note: "本地阶段未启用" }
  ];

  /* ---------- extended risk baseline (风控 §3–§9) ---------- */
  const P = (
    key: string,
    limit: number,
    cur: number | null,
    unit: string,
    sev: string,
    scope: string,
    opts?: { v?: string; dir?: "max" | "min" | "zero" | "ref" },
  ): RiskParam => {
    const o = opts || {};
    return {
      key, limit, cur, unit, sev, scope,
      v: o.v || "v1.0.4",
      dir: o.dir || "max",
      pct: cur === null || !limit ? null : Math.min(999, (cur / limit) * 100)
    };
  };

  const riskParams: RiskParam[] = riskParamsRaw.map((p) => ({
    key: p.key, limit: p.limit, cur: p.cur, unit: p.unit,
    sev: p.severity, scope: p.scope, v: p.v, dir: "max", pct: p.pct,
  }));

  const riskGroups = [
    {
      name: "资金与损失", source: "§3",
      items: [
        P("审计资金基准", 5000, 5182.87, "USDC", "REF", "GLOBAL", { dir: "ref" }),
        P("单笔实盘候选上限", 250, null, "USDC", "PAPER ONLY", "GLOBAL", { dir: "ref" }),
        P("单 Token 合并敞口", 500, 164.20, "USDC", "HARD STOP", "WETH"),
        P("单一场所资金使用率", 80, 62, "%", "SOFT BLOCK", "EXCHANGE"),
        P("全局日亏损", 50, 12.40, "USDC", "HARD STOP", "GLOBAL"),
        P("单策略日亏损", 25, 4.10, "USDC", "HARD STOP", "dex_triple"),
        P("最大权益回撤", 250, 49.09, "USDC", "HARD STOP", "GLOBAL"),
        P("同链待确认交易", 1, 0, "笔", "HARD STOP", "Arbitrum")
      ]
    },
    {
      name: "收益与成本门槛", source: "§4",
      items: [
        P("DEX 最低净利润", 2.50, 4.02, "USDC", "HARD STOP", "dex_triple", { dir: "min" }),
        P("Gas / 毛利占比", 25, 12.0, "%", "HARD STOP", "GLOBAL"),
        P("绝对 Gas 上限", 5.00, 1.18, "USDC", "HARD STOP", "Arbitrum"),
        P("单路线最大跳数", 4, 3, "跳", "SOFT BLOCK", "dex_triple"),
        P("CEX–DEX 最低净利润", 7.50, 9.64, "USDC", "HARD STOP", "cex_dex", { dir: "min" }),
        P("CEX 深度倍数要求", 10, 38.3, "倍", "HARD STOP", "ETH/USDT", { dir: "min" }),
        P("CEX 保护价偏离", 10, 3.4, "bps", "HARD STOP", "ETH/USDT"),
        P("DEX 价格冲击", 20, 10.4, "bps", "HARD STOP", "dex_triple"),
        P("预估收益安全折扣", 50, 50, "%", "PAPER ONLY", "GLOBAL", { dir: "ref" })
      ]
    },
    {
      name: "DEX 数据与执行", source: "§5",
      items: [
        P("报价区块差", 1, 0, "块", "HARD STOP", "GLOBAL"),
        P("报价最大时间", 2400, 312, "ms", "HARD STOP", "Arbitrum"),
        P("RPC 节点数", 2, 2, "个", "HARD STOP", "GLOBAL", { dir: "min" }),
        P("RPC 区块差异", 1, 0, "块", "HARD STOP", "Arbitrum"),
        P("RPC P95 预警", 500, 188, "ms", "WARNING", "Arbitrum"),
        P("RPC P95 硬阈值", 1000, 188, "ms", "HARD STOP", "Arbitrum"),
        P("连续 RPC 失败", 3, 0, "次", "HARD STOP", "GLOBAL", { dir: "zero" }),
        P("交易计划有效期", 30, 13, "s", "HARD STOP", "GLOBAL")
      ]
    },
    {
      name: "CEX 市场数据", source: "§6",
      items: [
        P("盘口最大年龄", 250, 96, "ms", "HARD STOP", "Binance"),
        P("盘口深度占用", 10, 6.2, "%", "SOFT BLOCK", "ETH/USDT"),
        P("本机与交易所时间差", 250, 38, "ms", "WARNING", "Binance"),
        P("盘口序列缺口", 0, 0, "个", "HARD STOP", "ETH/USDT", { dir: "zero" }),
        P("API 速率余量", 30, 64, "%", "WARNING", "Binance", { dir: "min" }),
        P("WS 断线", 0, 0, "次/日", "WARNING", "Binance", { dir: "zero" })
      ]
    },
    {
      name: "双腿对冲", source: "§7",
      items: [
        P("未对冲敞口", 25, 3.40, "USDC", "HARD STOP", "cex_dex"),
        P("最大未对冲时间", 2000, 842, "ms", "HARD STOP", "cex_dex"),
        P("对冲完成时限", 2000, 980, "ms", "HARD STOP", "cex_dex"),
        P("订单响应超时", 1000, null, "ms", "HARD STOP", "Binance", { dir: "ref" }),
        P("状态未知查询窗口", 2000, null, "ms", "HARD STOP", "Binance", { dir: "ref" }),
        P("自动重新报价次数", 1, 0, "次", "SOFT BLOCK", "cex_dex"),
        P("连续对冲异常", 2, 0, "次/10", "HARD STOP", "ETH/USDT", { dir: "zero" })
      ]
    },
    {
      name: "闪电贷与 Token", source: "§8–§9",
      items: [
        P("Provider 白名单", 1, 1, "个", "HARD STOP", "Aave V3", { dir: "min" }),
        P("单笔借款上限", 5000, 2500, "USDC", "HARD STOP", "Aave V3"),
        P("闪电贷费用", 9, 5, "bps", "HARD STOP", "Aave V3"),
        P("稳定币偏离预警", 0.5, 0.02, "%", "WARNING", "USDT"),
        P("稳定币偏离暂停", 1.0, 0.02, "%", "HARD STOP", "USDT"),
        P("未验证 Hook 数量", 0, 0, "个", "HARD STOP", "Uniswap V4", { dir: "zero" })
      ]
    }
  ];
  const circuitRules: CircuitRule[] = [
    { trigger: "全局日亏损达到 1%", scope: "全部策略", recover: "重新核对账本、损失原因和风险配置", last: "—" },
    { trigger: "最大回撤达到 5%", scope: "全局暂停", recover: "风险审查后重新启用", last: "—" },
    { trigger: "3 次 / 15 分钟 DEX 回滚", scope: "相关链或策略", recover: "至少等待 30 分钟并检查回滚原因", last: "09:47" },
    { trigger: "CEX–DEX 连续 2 次对冲异常 / 10 次尝试", scope: "相关交易对", recover: "完成订单、库存和账本核对", last: "—" },
    { trigger: "RPC 持续失联", scope: "对应链", recover: "双 RPC 状态重新一致", last: "11:02" },
    { trigger: "CEX 盘口不完整", scope: "对应交易对", recover: "Snapshot 与增量完整同步", last: "16:40" },
    { trigger: "订单状态未知", scope: "对应账户和交易对", recover: "查询并核对最终订单状态", last: "昨日 22:15" },
    { trigger: "库存账本不一致", scope: "对应策略", recover: "真实余额或 Paper 账本校准", last: "昨日 20:03" },
    { trigger: "Token / Pool / Adapter 风险", scope: "对应资产或协议", recover: "重新完成风险审查", last: "—" },
    { trigger: "合约权限或代码哈希异常", scope: "对应链合约", recover: "停止执行并完成安全检查", last: "—" }
  ];

  /* ---------- ledger accounts & flash loans (TDD §9.1) ---------- */
  const ledgerAccounts: LedgerAccount[] = [
    { name: "DEX Wallet Paper", type: "ASSET", balance: 3630.61, note: "Arbitrum + Base" },
    { name: "CEX Spot Paper", type: "ASSET", balance: 1552.09, note: "Binance" },
    { name: "Flash Loan Clearing", type: "CLEARING", balance: 0, note: "借款 / 偿还闭环" },
    { name: "Gas Expense", type: "EXPENSE", balance: -141.62, note: "含回滚 Gas" },
    { name: "CEX Fee Expense", type: "EXPENSE", balance: -58.44, note: "8 bps" },
    { name: "DEX Fee Expense", type: "EXPENSE", balance: -96.20, note: "池费" },
    { name: "Hedge Compensation", type: "EXPENSE", balance: -58.12, note: "残余敞口平仓" },
    { name: "Inventory Rebalance", type: "EXPENSE", balance: -21.40, note: "跨场所调拨" },
    { name: "Realized PnL", type: "EQUITY", balance: 182.63, note: "已平仓" },
    { name: "Unrealized PnL", type: "EQUITY", balance: 3.20, note: "持仓浮动" }
  ];

  const flashLoans: FlashLoan[] = [
    { id: "FL-2219", provider: "Aave V3", token: "WETH", amount: 0.8951, feeBps: 5, fee: 1.23, block: 372451889, state: "REPAID", tx: "0x4a91…c702" },
    { id: "FL-2218", provider: "Aave V3", token: "USDC", amount: 4000, feeBps: 5, fee: 2.00, block: 372451841, state: "REPAID", tx: "0x9f30…12b8" },
    { id: "FL-2217", provider: "ERC-3156 · Balancer", token: "USDC", amount: 2500, feeBps: 10, fee: 2.50, block: 372451802, state: "ROLLED_BACK", tx: "0x2c11…8d4e" }
  ];


  /* ---------- strategy configuration (strategy-scoped overrides) ---------- */
  const strategyConfig: Record<string, StrategyConfig> = {
    dex_triple: {
      id: "dex_triple", name: "DEX 原子三角套利", family: "DEX 原子",
      thesis: "同一笔 EVM 交易内完成三跳闭环，回到起始资产；任一跳不满足最低输出则整笔回滚。",
      mode: "PAPER", enabled: true, version: "cfg-2026.11.19-b4",
      kpi: [
        { label: "今日机会", value: "18", note: "检出 → 风控通过 2" },
        { label: "可执行", value: "2", note: "净利润均 ≥ 门槛" },
        { label: "今日净利", value: "+6.62", note: "已扣 Gas 与池费", tone: "pos" },
        { label: "胜率", value: "71.9%", note: "近 90 日 96 笔" }
      ],
      groups: [
        {
          name: "利润与成本门槛", note: "只能比全局更严格",
          items: [
            { key: "minNet", label: "最低净利润", value: 2.50, unit: "USDC", min: 1, max: 20, step: 0.5, global: 2.00 },
            { key: "gasRatio", label: "Gas / 毛利上限", value: 20, unit: "%", min: 5, max: 25, step: 1, global: 25 },
            { key: "competition", label: "竞争折价", value: 15, unit: "%", min: 0, max: 50, step: 5, global: 0 }
          ]
        },
        {
          name: "规模与容量",
          items: [
            { key: "maxNotional", label: "单笔名义上限", value: 2500, unit: "USDC", min: 250, max: 10000, step: 250, global: 5000 },
            { key: "maxHops", label: "最大跳数", value: 3, unit: "跳", min: 2, max: 4, step: 1, global: 4 },
            { key: "flashMax", label: "闪电贷单笔上限", value: 2500, unit: "USDC", min: 0, max: 5000, step: 250, global: 5000 }
          ]
        },
        {
          name: "数据与时效",
          items: [
            { key: "quoteAge", label: "报价最大年龄", value: 2400, unit: "ms", min: 500, max: 3000, step: 100, global: 2400 },
            { key: "blockDelta", label: "报价区块差", value: 1, unit: "块", min: 0, max: 3, step: 1, global: 1 },
            { key: "cooldown", label: "回滚后冷却", value: 30, unit: "s", min: 0, max: 600, step: 10, global: 0 }
          ]
        },
        {
          name: "执行方式",
          items: [
            { key: "minOutBuffer", label: "最低输出缓冲", value: 25, unit: "bps", min: 5, max: 100, step: 5, global: 20 },
            { key: "planTTL", label: "计划有效期", value: 20, unit: "s", min: 5, max: 30, step: 5, global: 30 }
          ]
        }
      ],
      toggles: [
        { key: "flash", label: "允许闪电贷", on: true, note: "Aave V3 单资产", locked: false },
        { key: "mixed", label: "允许自有资金 + 闪电贷混合", on: true, note: "", locked: false },
        { key: "autoCooldown", label: "回滚后自动冷却该路线", on: true, note: "", locked: false },
        { key: "v4", label: "允许 V4 / Hook 池", on: false, note: "未独立审计前硬禁止", locked: true }
      ],
      history: [
        { at: "11-19 14:02", by: "operator", change: "最低净利润 2.00 → 2.50" },
        { at: "11-18 20:31", by: "operator", change: "竞争折价 10% → 15%" },
        { at: "11-16 09:12", by: "system", change: "参数冻结 cfg-2026.11.16-b2" }
      ]
    },
    dex_cross: {
      id: "dex_cross", name: "DEX 原子跨池套利", family: "DEX 原子",
      thesis: "同一链上利用不同 DEX / 不同池的定价差异完成原子闭环，每跳独立校验池身份与费率。",
      mode: "PAPER", enabled: true, version: "cfg-2026.11.19-b4",
      kpi: [
        { label: "今日机会", value: "12", note: "检出 → 风控通过 1" },
        { label: "可执行", value: "1", note: "竞争评分 ≥ 60" },
        { label: "今日净利", value: "+3.75", note: "已扣 Gas 与池费", tone: "pos" },
        { label: "胜率", value: "64.1%", note: "近 90 日 64 笔" }
      ],
      groups: [
        {
          name: "利润与成本门槛", note: "跨池价差通常更薄",
          items: [
            { key: "minNet", label: "最低净利润", value: 2.00, unit: "USDC", min: 1, max: 20, step: 0.5, global: 2.00 },
            { key: "impactMax", label: "价格冲击上限", value: 20, unit: "bps", min: 5, max: 50, step: 5, global: 20 },
            { key: "gasRatio", label: "Gas / 毛利上限", value: 20, unit: "%", min: 5, max: 25, step: 1, global: 25 }
          ]
        },
        {
          name: "规模与容量",
          items: [
            { key: "maxNotional", label: "单笔名义上限", value: 1500, unit: "USDC", min: 250, max: 10000, step: 250, global: 5000 },
            { key: "maxHops", label: "最大跳数", value: 2, unit: "跳", min: 2, max: 4, step: 1, global: 4 },
            { key: "minTvl", label: "池 TVL 下限", value: 500000, unit: "USDC", min: 100000, max: 2000000, step: 100000, global: 0 }
          ]
        },
        {
          name: "数据与时效",
          items: [
            { key: "quoteAge", label: "报价最大年龄", value: 2400, unit: "ms", min: 500, max: 3000, step: 100, global: 2400 },
            { key: "poolStateMax", label: "池状态最大区块差", value: 1, unit: "块", min: 0, max: 3, step: 1, global: 1 },
            { key: "cooldown", label: "回滚后冷却", value: 60, unit: "s", min: 0, max: 600, step: 10, global: 0 }
          ]
        },
        {
          name: "执行方式",
          items: [
            { key: "minOutBuffer", label: "最低输出缓冲", value: 30, unit: "bps", min: 5, max: 100, step: 5, global: 20 },
            { key: "planTTL", label: "计划有效期", value: 20, unit: "s", min: 5, max: 30, step: 5, global: 30 }
          ]
        }
      ],
      toggles: [
        { key: "stable", label: "允许 StableSwap 池", on: true, note: "Curve 类", locked: false },
        { key: "weighted", label: "允许 Weighted Pool", on: true, note: "Balancer 类", locked: false },
        { key: "v2", label: "允许 V2 类池", on: true, note: "", locked: false },
        { key: "v4", label: "允许 V4 / Hook 池", on: false, note: "未独立审计前硬禁止", locked: true }
      ],
      history: [
        { at: "11-19 10:22", by: "operator", change: "池 TVL 下限 300k → 500k" },
        { at: "11-17 16:40", by: "operator", change: "价格冲击上限 25 → 20 bps" }
      ]
    },
    cex_dex: {
      id: "cex_dex", name: "CEX–DEX 非原子套利", family: "链外对冲",
      thesis: "两条腿分别成交，不存在共同原子提交；任一腿的部分成交、回滚或状态未知都必须显式处置。",
      mode: "PAPER", enabled: true, version: "cfg-2026.11.19-b4",
      kpi: [
        { label: "今日机会", value: "9", note: "检出 → 风控通过 1" },
        { label: "未对冲敞口", value: "3.40", note: "上限 25.00 USDC", tone: "warn" },
        { label: "今日净利", value: "+7.58", note: "已扣双边费用与准备金", tone: "pos" },
        { label: "对冲完成", value: "25.0%", note: "今日 8 笔执行", tone: "neg" }
      ],
      groups: [
        {
          name: "利润与成本门槛",
          note: "保守净利润，含延迟与补偿准备金",
          items: [
            { key: "minNet", label: "最低保守净利润", value: 7.50, unit: "USDC", min: 5, max: 50, step: 0.5, global: 5.00 },
            { key: "depthMultiple", label: "深度倍数要求", value: 10, unit: "倍", min: 5, max: 50, step: 1, global: 10 },
            { key: "delayReserve", label: "延迟风险准备金", value: 35, unit: "bps", min: 0, max: 100, step: 5, global: 0 }
          ]
        },
        {
          name: "规模与容量",
          items: [
            { key: "maxNotional", label: "单笔名义上限", value: 2500, unit: "USDC", min: 250, max: 10000, step: 250, global: 5000 },
            { key: "maxExposure", label: "未对冲敞口上限", value: 25, unit: "USDC", min: 5, max: 100, step: 5, global: 25 },
            { key: "rebalanceBudget", label: "再平衡预算", value: 2.00, unit: "USDC", min: 0, max: 20, step: 0.5, global: 0 }
          ]
        },
        {
          name: "数据与时效",
          items: [
            { key: "bookAge", label: "盘口最大年龄", value: 250, unit: "ms", min: 50, max: 500, step: 25, global: 250 },
            { key: "maxUnhedgedSec", label: "最大未对冲时间", value: 2.0, unit: "s", min: 0.5, max: 5, step: 0.5, global: 2.0 },
            { key: "queryWindow", label: "状态未知查询窗口", value: 2.0, unit: "s", min: 0.5, max: 5, step: 0.5, global: 2.0 }
          ]
        },
        {
          name: "执行方式",
          items: [
            { key: "protectBps", label: "保护价偏离", value: 10, unit: "bps", min: 1, max: 30, step: 1, global: 10 },
            { key: "maxReprice", label: "最大重新报价", value: 1, unit: "次", min: 0, max: 3, step: 1, global: 1 },
            { key: "compLossMax", label: "补偿最大亏损", value: 3.00, unit: "USDC", min: 1, max: 20, step: 0.5, global: 0 }
          ]
        }
      ],
      toggles: [
        { key: "dexFirst", label: "允许 DEX 先行", on: true, note: "A 组：链上先成交", locked: false },
        { key: "cexFirst", label: "允许 CEX 先行", on: true, note: "B 组：盘口先成交", locked: false },
        { key: "partialReplan", label: "部分成交后按实际量重算另一腿", on: true, note: "硬性要求", locked: true },
        { key: "autoComp", label: "允许自动补偿", on: true, note: "受补偿最大亏损约束", locked: false },
        { key: "noResend", label: "状态未知时禁止重发", on: true, note: "硬性要求", locked: true },
        { key: "perp", label: "允许永续 / 杠杆", on: false, note: "第一阶段不启用", locked: true }
      ],
      history: [
        { at: "11-19 16:41", by: "operator", change: "最低保守净利润 7.00 → 7.50" },
        { at: "11-18 13:05", by: "operator", change: "补偿最大亏损 2.00 → 3.00" },
        { at: "11-15 09:30", by: "system", change: "参数冻结 cfg-2026.11.15-b1" }
      ]
    }
  };

  /* ---------- DEX route search space (this round) ---------- */
  const routeSpace = {
    tokens: 4, pools: 326, hops: 3,
    candidates: 21776, pruned: 1842, simulated: 312, passed: 2,
    byStrategy: { dex_triple: { candidates: 12840, simulated: 184, passed: 1 }, dex_cross: { candidates: 8936, simulated: 128, passed: 1 } }
  };


  const model = {
    px, venues, equity, startEquity, series: curve, maxDD, maxDDusd, todayPnl, weekPnl,
    opps, STRATEGY, CHAIN, executions, EXEC_STATES, ledger, LEDGER_TYPES,
    riskParams, riskEvents, scopes, auditJobs, services, chains, CHAIN_STAGES,
    dexProtocols, exchanges, instruments, orderbooks, health,
    pools, tokens, adapters, orderbook, rpcNodes, cexIntegrity, archiveCoverage,
    clockSkew, dataGaps, contractComponents, contractTests, contractEvents,
    riskGroups, circuitRules, ledgerAccounts, flashLoans, strategyConfig, routeSpace,
    scanner: {
      state: "RUNNING", started: "09:12:04", uptime: "8h 14m 37s", quotes: 1284,
      detected: 37, actionable: 3, rejected: 4, watch: 3, expired: 1, executed: 1,
      chains: 3, protocols: 7, pools: 326
    }
  };

export const data: DataModel = model;
