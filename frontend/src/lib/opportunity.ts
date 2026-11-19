import { fmt } from "./format";
import type { Leg, Opportunity } from "../types";

const PRICE: Record<string, number> = {
  WETH: 2793.1,
  ETH: 2793.1,
  USDC: 1,
  USDT: 1.0002,
  DAI: 1.0001,
  ARB: 0.612,
  WBTC: 68640,
};

const tp = (sym: string) => PRICE[sym] ?? 1;

const qty = (sym: string, v: number) =>
  `${fmt.num(v, sym === "WETH" || sym === "ETH" || sym === "WBTC" ? 4 : 2)} ${sym}`;

export function legsFor(o: Opportunity): Leg[] {
  if (o.legs) return o.legs;
  let toks: string[];
  let pools: string[];
  if (o.strategy === "cex_dex" && o.symbol) {
    const [base = "", quote = ""] = o.symbol.split("/");
    toks = [quote, base, quote];
    pools = o.venue.split("·").map((s) => s.trim());
    if (/DEX\s*买入/.test(o.pair)) pools = pools.slice().reverse();
  } else {
    toks = o.pair.split("→").map((s) => s.trim());
    pools = o.venue.split("·").map((s) => s.trim());
  }
  const n = Math.max(1, toks.length - 1);
  const out: Leg[] = [];
  for (let i = 0; i < n; i++) {
    const from = toks[i];
    const to = toks[i + 1];
    const pool = pools[Math.min(i, pools.length - 1)];
    const vIn = o.notional + (o.theo * i) / n;
    const vOut = o.notional + (o.theo * (i + 1)) / n;
    const isCex = /binance|okx|bybit/i.test(pool);
    out.push({
      n: i + 1,
      from,
      to,
      pool,
      fee: isCex ? "8 bps" : /curve|balancer/i.test(pool) ? "0.01%" : /v2|sushi|aerodrome/i.test(pool) ? "0.30%" : "0.05%",
      inQty: qty(from, vIn / tp(from)),
      outQty: qty(to, vOut / tp(to)),
      minOut: qty(to, (vOut / tp(to)) * 0.998),
      impact: `${fmt.num(((o.facts.impactBps || 6) * (i + 1)) / n, 1)} bps`,
    });
  }
  return out;
}

export interface CapacityPoint {
  notional: number;
  net: number;
}

export function capacityFor(o: Opportunity): CapacityPoint[] {
  const steps = [250, 500, 1000, 2500, 5000, 10000];
  const fixed = o.costs.gas + o.costs.priority;
  const varCost = Math.max(0, o.costTotal - fixed);
  return steps.map((N) => {
    const x = N / o.notional;
    const depth = x <= 1 ? 1 : 1 / (1 + 0.22 * Math.pow(x - 1, 1.25));
    const gross = o.gross * x * depth;
    const cost = fixed + varCost * x;
    return { notional: N, net: gross - cost };
  });
}

export interface FailureModel {
  name: string;
  prob: number;
  loss: number;
  note: string;
}

export function failureModels(o: Opportunity): FailureModel[] {
  const g = o.costs.gas + o.costs.priority;
  if (o.strategy === "cex_dex") {
    return [
      { name: "CEX 部分成交 62%", prob: 18.4, loss: -(o.costs.hedge + o.costs.slippage * 0.8), note: "按实际成交量重算另一腿，残余按新盘口补偿" },
      { name: "DEX 腿回滚", prob: 6.1, loss: -g, note: "整笔回滚；CEX 侧按补偿流程反向平仓" },
      { name: "对冲失败", prob: 2.3, loss: -(o.costs.slippage * 2 + o.costs.hedge), note: "DEX 已成交而 CEX 未成交，追价受限价约束" },
    ];
  }
  return [
    { name: "整笔回滚", prob: 6.1, loss: -g, note: "任一腿最低输出不满足，损失为 Gas" },
    { name: "被夹击 / 抢先", prob: 9.4, loss: -(o.costs.slippage + g), note: "区块内竞争导致成交价劣化" },
    { name: "池状态不一致", prob: 2.2, loss: -g, note: "读取到过期状态，拒绝成交并重新报价" },
  ];
}

export interface ScorePart {
  name: string;
  part: number;
  weight: number;
}

export function scoreFor(o: Opportunity): ScorePart[] {
  const weights = [25, 20, 15, 15, 10, 10, 5];
  const names = ["保守净利润", "模拟可信度", "流动性容量", "执行竞争", "数据新鲜度", "Token / 协议风险", "历史执行表现"];
  const ratio = o.riskScore / 100;
  const parts = weights.map((w) => Math.round(w * ratio * 1.02));
  parts[0] += o.riskScore - parts.reduce((a, b) => a + b, 0);
  return parts.map((p, i) => ({ name: names[i], part: Math.max(0, p), weight: weights[i] }));
}

export interface Gate {
  name: string;
  detail: string;
  val: string;
  ok: boolean;
}

export function gatesFor(o: Opportunity, chains: { name: string; status: string; stage: number }[]): Gate[] {
  const g: Gate[] = [];
  const minP = o.minProfit;
  g.push({
    name: "最低净利润",
    detail: o.strategy === "cex_dex" ? "max(5 USDC, 名义 × 30 bps)" : "max(2 USDC, 名义 × 10 bps)",
    val: `${fmt.signed(o.net)} / ${fmt.num(minP)}`,
    ok: o.net >= minP,
  });
  g.push({ name: "Gas / 毛利 ≤ 25%", detail: "硬性成本占比上限", val: fmt.pct(o.gasRatio, 1), ok: o.gasRatio <= 25 });
  g.push({ name: "单笔 Gas ≤ 5.00 USDC", detail: "按链单独校准", val: fmt.num(o.costs.gas), ok: o.costs.gas <= 5 });
  g.push({ name: "DEX 路线跳数 ≤ 4", detail: "超过需额外验证", val: `${o.facts.hops} 跳`, ok: o.facts.hops <= 4 });
  g.push({ name: "报价区块差 ≤ 1", detail: "禁止用区块结束状态代替交易前状态", val: `${o.blockDelta} 块`, ok: o.blockDelta <= 1 });
  g.push({ name: "报价年龄在有效期内", detail: "min(2 × 区块时间, 3 s)", val: fmt.age(o.quoteAgeMs), ok: o.quoteAgeMs <= 2400 });
  g.push({ name: "价格冲击 ≤ 20 bps", detail: "全路线累计", val: `${fmt.num(o.facts.impactBps, 1)} bps`, ok: o.facts.impactBps <= 20 });
  g.push({ name: "主备 RPC 一致", detail: "区块 Hash 与高度", val: o.facts.rpcOk ? "一致" : "不一致", ok: o.facts.rpcOk });
  g.push({ name: "数据质量 ≥ B", detail: "来源、覆盖与缺口校验", val: `Q${o.quality}`, ok: o.quality !== "C" });
  g.push({ name: "Token / Pool 白名单", detail: "未验证资产不得自动执行", val: o.facts.tokenOk ? "在册" : "未登记", ok: o.facts.tokenOk });
  g.push({ name: "库存充足", detail: "按计划数量 + 安全缓冲", val: o.facts.inventoryOk ? "充足" : "不足", ok: o.facts.inventoryOk });
  if (o.strategy !== "cex_dex") {
    g.push({ name: "闪电贷 Provider 白名单", detail: "Aave V3 · 单资产", val: o.facts.providerOk ? "在册" : "未登记", ok: o.facts.providerOk });
  } else {
    g.push({
      name: "CEX 盘口完整性",
      detail: "Snapshot + 增量无缺口，年龄 ≤ 250 ms",
      val: fmt.age(o.facts.bookAgeMs ?? 0),
      ok: (o.facts.bookAgeMs ?? 0) <= 250,
    });
    g.push({
      name: "盘口深度占用 ≤ 10%",
      detail: "允许区间内可见深度",
      val: fmt.pct(o.facts.depthUsage ?? 0, 1),
      ok: (o.facts.depthUsage ?? 0) <= 10,
    });
  }
  const chain = chains.find((c) => c.name.toLowerCase() === o.chain);
  g.push({ name: "链状态达 PAPER_READY", detail: "阶段准入门槛", val: chain ? chain.status : "—", ok: chain ? chain.stage >= 4 : false });
  return g;
}
