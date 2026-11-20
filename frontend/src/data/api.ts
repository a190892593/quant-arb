/**
 * ARBX API client + mappers.
 *
 * 契约：后端 `/api/*`（TDD §11）→ `types.ts` 领域模型。
 * 只做形状转换，不臆造数值；后端缺失的集合保持演示数据（由 `index.ts` 的 patch 合并策略决定）。
 *
 * 写操作（扫描开关、风控暂停、执行机会）集中在文件末尾，供 `store.ts` / 视图调用。
 */
import type {
  Adapter,
  AuditJob,
  BookIntegrity,
  ChainInfo,
  ChainKey,
  CircuitRule,
  ContractComponent,
  ContractEvent,
  ContractTest,
  DataModel,
  DexEngineParams,
  DexEngineResponse,
  DexGraphResponse,
  DexProtocol,
  EquityPoint,
  Exchange,
  Execution,
  ExecutionLeg,
  FlashLoan,
  GasEstimate,
  GraphStats,
  PricesResponse,
  InputOptimization,
  InputPoint,
  Instrument,
  LedgerAccount,
  LedgerEntry,
  Leg,
  Opportunity,
  OpportunityCosts,
  Pool,
  RiskEvent,
  RiskGroup,
  RiskParam,
  RiskViolationRecord,
  RouteCacheStats,
  RouteLegRecord,
  RouteLifecycle,
  RouteScanRecord,
  RouteScanResponse,
  RouteType,
  RpcNode,
  ScanRunStats,
  SearchConfigResponse,
  SearchParamsConfig,
  SearchTokenEntry,
  DiscoveryStatus,
  SourceConfig,
  SourcePresetResponse,
  Service,
  SimulationLevel,
  SimulationOutcome,
  SimulationStatus,
  StrategyId,
  TokenRow,
  Venue,
} from "../types";

const BASE = "/api";

type Raw = Record<string, unknown>;

/* ------------------------------- primitives ------------------------------- */

export function num(v: unknown, d = 0): number {
  if (typeof v === "number") return Number.isFinite(v) ? v : d;
  if (typeof v === "string") {
    const n = Number(v);
    return Number.isFinite(n) ? n : d;
  }
  return d;
}
export const str = (v: unknown, d = ""): string =>
  typeof v === "string" ? v : typeof v === "number" ? String(v) : d;
const bool = (v: unknown, d = false): boolean => (typeof v === "boolean" ? v : d);
const arr = <T = Raw>(v: unknown): T[] => (Array.isArray(v) ? (v as T[]) : []);
const obj = (v: unknown): Raw => (v && typeof v === "object" && !Array.isArray(v) ? (v as Raw) : {});
const clock = (v: unknown, withMs = false): string => {
  const s = str(v);
  if (!s) return "—";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return s;
  const p = (n: number, l = 2) => String(n).padStart(l, "0");
  const base = `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  return withMs ? `${base}.${p(d.getMilliseconds(), 3)}` : base;
};
const msHuman = (ms: number): string => {
  if (!ms || ms < 0) return "—";
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return `${h ? `${h}h ` : ""}${m}m ${s % 60}s`;
};
const shortId = (id: string, prefix: string): string =>
  `${prefix}-${id.replace(/^[a-z]+_/, "").slice(0, 8).toUpperCase()}`;
const round = (v: number, dp = 2): number => {
  const f = 10 ** dp;
  return Math.round(v * f) / f;
};

const CHAIN_BY_ID: Record<number, ChainKey> = { 42161: "arbitrum", 8453: "base", 1: "ethereum", 137: "polygon" };
const chainOf = (id: unknown): ChainKey => CHAIN_BY_ID[num(id)] ?? "arbitrum";
const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);
const pairOf = (symbol: string): string => {
  const s = symbol.toUpperCase();
  for (const q of ["USDT", "USDC", "USD"]) {
    if (s.endsWith(q) && s.length > q.length) return `${s.slice(0, -q.length)}/${q}`;
  }
  return s;
};
const poolName = (id: string): string => {
  const s = id ?? "";
  if (s.startsWith("uni-v3")) return "Uniswap V3";
  if (s.startsWith("uni-v2")) return "Uniswap V2";
  if (s.startsWith("sushi")) return "SushiSwap";
  if (s.startsWith("curve")) return "Curve";
  if (s.startsWith("bal")) return "Balancer";
  if (s.startsWith("aero")) return "Aerodrome";
  return s || "Pool";
};
const qtyStr = (v: unknown): string => {
  const n = num(v);
  return n.toLocaleString("en-US", { maximumFractionDigits: n < 1 ? 6 : 4 });
};

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { headers: { accept: "application/json" } });
  if (!res.ok) {
    let message = `${path} → HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      /* 非 JSON 错误响应 */
    }
    throw new Error(message);
  }
  return (await res.json()) as T;
}
const safe = <T>(p: Promise<T>): Promise<T | null> => p.catch(() => null);

/* ------------------------------- opportunities ---------------------------- */

const COST_KEY: Record<string, keyof OpportunityCosts> = {
  dex_fee: "dexFee",
  cex_fee: "cexFee",
  gas: "gas",
  priority_fee: "priority",
  flash_fee: "flash",
  price_impact: "slippage",
  slippage: "slippage",
  delay_reserve: "slippage",
  mev_reserve: "slippage",
  hedge_compensation: "hedge",
  rebalance: "rebalance",
  other: "rebalance",
};

function mapOpportunity(o: Raw): Opportunity {
  const costs: OpportunityCosts = {
    dexFee: 0,
    cexFee: 0,
    gas: 0,
    priority: 0,
    flash: 0,
    slippage: 0,
    hedge: 0,
    rebalance: 0,
  };
  for (const item of arr<Raw>(obj(o.costs).items)) {
    const k = COST_KEY[str(item.key)];
    if (k) costs[k] += num(item.amount);
  }
  const costTotal = num(o.total_cost, Object.values(costs).reduce((a, b) => a + b, 0));

  const route = obj(o.dex_route);
  const legsRaw = arr<Raw>(route.legs);
  const notional = num(o.target_notional);
  const net = num(o.conservative_net_profit);
  const gross = num(o.executable_gross_profit);
  const strategy = str(o.strategy_type, "dex_triple") as StrategyId;
  const chain = chainOf(o.chain_id);
  const chainLabel = str(o.chain_name, "Arbitrum");
  const symbol = o.cex_symbol ? pairOf(str(o.cex_symbol)) : undefined;
  const cex = o.cex_exchange ? cap(str(o.cex_exchange)) : undefined;

  const tokenPath = legsRaw.length
    ? [str(legsRaw[0].token_in), ...legsRaw.map((l) => str(l.token_out))].join(" → ")
    : symbol ?? "—";

  const venue =
    Array.from(new Set(legsRaw.map((l) => poolName(str(l.pool_id))))).join(" · ") || chainLabel;
  const pairLabel =
    strategy === "cex_dex"
      ? str(o.direction) === "DEX_BUY_CEX_SELL"
        ? "DEX 买入 → CEX 卖出"
        : "CEX 买入 → DEX 卖出"
      : tokenPath;

  const legs: Leg[] = legsRaw.map((l, i) => ({
    n: i + 1,
    from: str(l.token_in),
    to: str(l.token_out),
    pool: poolName(str(l.pool_id)),
    pair: `${str(l.token_in)}/${str(l.token_out)}`,
    fee: `${(num(l.fee_bps) / 100).toFixed(2)}%`,
    inQty: `${qtyStr(l.amount_in)} ${str(l.token_in)}`,
    outQty: `${qtyStr(l.expected_out)} ${str(l.token_out)}`,
    minOut: qtyStr(l.min_out),
    impact: `${num(l.price_impact_bps).toFixed(1)} bps`,
  }));

  const rs = obj(o.risk_score);
  const components = arr<Raw>(rs.components);
  const points = components.reduce((s, c) => s + num(c.points), 0);
  const riskScore = Math.round(num(rs.total, points));
  const competition = components.find((c) => str(c.key) === "competition");
  const impact = num(route.price_impact_bps);
  const rejections = arr<Raw>(o.rejections);
  const depthRejection = rejections.find((r) => str(r.key) === "cex_depth_usage_max_pct");
  const inventory = [...arr<Raw>(o.required_dex_inventory), ...arr<Raw>(o.required_cex_inventory)];
  const m2raw = obj(o.m2);

  return {
    id: shortId(str(o.opportunity_id), "OPP"),
    rawId: str(o.opportunity_id),
    strategy,
    chain,
    cex,
    symbol,
    venue,
    pair: pairLabel,
    notional,
    theo: num(obj(o.scenario).theoretical_usd, notional * (num(o.theoretical_edge_bps) / 10000)),
    gross,
    costs,
    stressNet: num(o.stress_net_profit),
    riskScore,
    quality: (str(o.data_quality, "B") as Opportunity["quality"]),
    status: str(o.status, "WATCH") as Opportunity["status"],
    ageMs: num(o.quote_age_ms),
    model: str(o.model_level, "M2") as Opportunity["model"],
    m2: o.m2
      ? {
          passed: bool(m2raw.passed),
          block: num(m2raw.block),
          amountIn: num(m2raw.amount_in),
          m1Out: num(m2raw.m1_out),
          m2Out: num(m2raw.m2_out),
          diffBps: num(m2raw.diff_bps),
          gasUsed: num(m2raw.gas_used),
          elapsedMs: num(m2raw.elapsed_ms),
          detail: str(m2raw.detail),
        }
      : undefined,
    rejections: arr<Raw>(o.rejections).map((r) => ({
      key: str(r.key),
      label: str(r.label),
      severity: str(r.severity),
      limit: num(r.limit),
      actual: num(r.actual),
      unit: str(r.unit),
      message: str(r.message),
    })),
    block: num(o.quote_block_number),
    blockDelta: 0,
    quoteAgeMs: num(o.quote_age_ms),
    facts: {
      hops: legs.length,
      impactBps: impact,
      rpcOk: str(o.data_quality) !== "INVALID",
      tokenOk: str(o.data_quality) !== "INVALID",
      providerOk: true,
      inventoryOk: inventory.every((r) => num(obj(r).available) >= num(obj(r).required)),
      competition: Math.round(num(competition?.points)),
      depthUsage: depthRejection ? num(depthRejection.actual) : undefined,
      bookAgeMs: num(o.book_age_ms),
    },
    reasons: rejections.map((r) => str(r.message)),
    legs,
    direction: o.direction ? str(o.direction) : null,
    hedgeOrder: o.hedge_order ? str(o.hedge_order) : null,
    expiresAt: str(o.expires_at),
    detectedAt: str(o.detected_at),
    capacity: num(o.capacity_usd),
    costTotal,
    net,
    minProfit: Math.max(2, notional * 0.001),
    netBps: notional ? (net / notional) * 10000 : 0,
    theoBps: num(o.theoretical_edge_bps),
    gasRatio: gross > 0 ? Math.max(0, ((costs.gas + costs.priority) / gross) * 100) : 0,
  };
}

/* --------------------------------- executions ----------------------------- */

function mapExecution(e: Raw): Execution {
  const legs: ExecutionLeg[] = arr<Raw>(e.legs).map((l) => ({
    side: str(l.venue_kind) === "CEX" ? "CEX" : "DEX",
    venue: str(l.venue),
    state: str(l.status),
    submitted: clock(l.submitted_at, true),
    filled: clock(l.confirmed_at, true),
    qty: `${qtyStr(l.qty_filled)} / ${qtyStr(l.qty_requested)} ${str(l.symbol)}`,
    price: num(l.avg_price).toFixed(4),
    fee: `${num(l.fee_amount).toFixed(4)} ${str(l.fee_asset, "USDC")}`,
    slip: "—",
  }));
  const events = arr<Raw>(e.events);
  const lastEvent = events[events.length - 1];
  const exposure = num(obj(e.exposure).usd_value);
  const strategy = str(e.strategy_type, "dex_triple") as StrategyId;
  const hedge = str(e.hedge_order);
  return {
    id: shortId(str(e.execution_id), "EX"),
    oppId: shortId(str(e.opportunity_id), "OPP"),
    strategy,
    chain: chainOf(e.chain_id),
    cex: undefined,
    symbol: e.symbol ? pairOf(str(e.symbol)) : undefined,
    pair:
      strategy === "cex_dex"
        ? str(e.direction) === "DEX_BUY_CEX_SELL"
          ? "DEX 买入 → CEX 卖出"
          : "CEX 买入 → DEX 卖出"
        : `${arr<Raw>(e.legs).map((l) => str(l.symbol)).join(" → ")}（原子闭环）`,
    notional: num(e.target_notional),
    state: str(e.state, "DETECTED"),
    order: hedge === "CEX_FIRST" ? "CEX 先行" : hedge === "DEX_FIRST" ? "DEX 先行" : "DEX 原子",
    startedAt: str(e.created_at),
    duration: num(e.exposure_ms),
    exposure: Math.abs(exposure),
    exposureMs: num(e.exposure_ms),
    netPnl: num(e.net_pnl),
    statusNote: str(lastEvent?.message, str(e.state)),
    legs,
    revert: obj(e.dex_tx).revert_reason ? str(obj(e.dex_tx).revert_reason) : undefined,
    ledger: arr<Raw>(obj(e.costs).items).map((i) => ({
      account: str(i.label),
      delta: round(num(i.amount), 4).toFixed(4),
    })),
    exposureSeries: [],
  };
}

/* -------------------------------- performance ----------------------------- */

function mapSeries(rows: Raw[]): EquityPoint[] {
  let peak = -Infinity;
  return rows.map((p, i) => {
    const equity = num(p.equity);
    peak = Math.max(peak, equity);
    return {
      i,
      equity,
      date: new Date(str(p.ts)),
      peak,
      dd: peak > 0 ? (peak - equity) / peak : 0,
    };
  });
}

/* ---------------------------------- markets ------------------------------- */

interface Ticker {
  exchange: string;
  symbol: string;
  bid: number;
  ask: number;
  age: number;
  valid: boolean;
  seq: number;
  ts: string;
}

function readPrices(raw: unknown): { prices: Record<string, number>; tickers: Ticker[] } {
  const root = obj(raw);
  const prices: Record<string, number> = {};
  const src = root.prices;
  if (src && typeof src === "object" && !Array.isArray(src)) {
    for (const [k, v] of Object.entries(src as Record<string, unknown>)) prices[k.toUpperCase()] = num(v);
  }
  const tickers: Ticker[] = arr<Raw>(root.tickers).map((t) => ({
    exchange: str(t.exchange, "binance"),
    symbol: str(t.symbol).toUpperCase(),
    bid: num(t.bid),
    ask: num(t.ask),
    age: num(t.age_ms),
    valid: bool(t.valid, true),
    seq: num(t.seq),
    ts: str(t.ts),
  }));
  return { prices, tickers };
}

function mapInstrument(t: Ticker, meta: Raw | undefined): Instrument {
  const symbol = t.symbol;
  const quote = symbol.endsWith("USDC") ? "USDC" : "USDT";
  const base = symbol.slice(0, -quote.length) || symbol;
  return {
    symbol: pairOf(symbol),
    base,
    quote,
    exchange: cap(t.exchange),
    state: t.valid ? "TRADING" : "STALE",
    tradable: t.valid,
    bid: t.bid,
    ask: t.ask,
    spread: t.bid > 0 ? ((t.ask - t.bid) / t.bid) * 10000 : 0,
    depth10: 0,
    age: t.age,
    integrity: t.valid ? "VALID" : "INVALID",
    seq: t.seq,
    tick: num(meta?.price_tick, 0.01),
    step: num(meta?.qty_step, 0.0001),
    minNotional: num(meta?.min_notional, 5),
    maker: num(meta?.maker_bps),
    taker: num(meta?.taker_bps, 10),
    pricePrec: 2,
    qtyPrec: 4,
    depthScale: 1,
    recvTs: clock(t.ts, true),
  };
}

/* --------------------------------- risk ----------------------------------- */

interface Runtime {
  equity: number;
  dailyLoss: number;
  drawdown: number;
  maxBookAge: number;
  maxQuoteAge: number;
  maxGas: number;
  maxGasRatio: number;
  maxImpact: number;
  maxDepthUsage: number;
  maxExposure: number;
  maxExposureMs: number;
}

const EQUITY_PCT_KEYS = new Set(["global_daily_loss_pct", "max_drawdown_pct", "unhedged_exposure_max_pct"]);

function runtimeOf(equity: number, risk: Raw, opps: Opportunity[], execs: Execution[], books: Raw[]): Runtime {
  return {
    equity,
    dailyLoss: num(risk.account_daily_loss),
    drawdown: num(risk.max_drawdown),
    maxBookAge: books.length ? Math.max(...books.map((b) => num(b.age_ms))) : 0,
    maxQuoteAge: opps.length ? Math.max(...opps.map((o) => o.quoteAgeMs)) : 0,
    maxGas: opps.length ? Math.max(...opps.map((o) => o.costs.gas)) : 0,
    maxGasRatio: opps.length ? Math.max(...opps.map((o) => o.gasRatio)) : 0,
    maxImpact: opps.length ? Math.max(...opps.map((o) => o.facts.impactBps)) : 0,
    maxDepthUsage: opps.length ? Math.max(0, ...opps.map((o) => o.facts.depthUsage ?? 0)) : 0,
    maxExposure: execs.length ? Math.max(0, ...execs.map((e) => e.exposure)) : 0,
    maxExposureMs: execs.length ? Math.max(0, ...execs.map((e) => e.exposureMs)) : 0,
  };
}

function runtimeValue(key: string, rt: Runtime): number | null {
  switch (key) {
    case "audit_capital_usd":
      return rt.equity;
    case "global_daily_loss_pct":
      return rt.dailyLoss;
    case "max_drawdown_pct":
      return rt.drawdown;
    case "book_max_age_ms":
      return rt.maxBookAge;
    case "dex_quote_max_age_ms":
      return rt.maxQuoteAge;
    case "gas_abs_max_usd":
      return rt.maxGas;
    case "gas_profit_ratio_max_pct":
      return rt.maxGasRatio;
    case "dex_price_impact_max_bps":
    case "cex_dex_price_impact_max_bps":
      return rt.maxImpact;
    case "book_depth_usage_max_pct":
    case "cex_depth_usage_max_pct":
      return rt.maxDepthUsage;
    case "unhedged_exposure_max_pct":
      return rt.maxExposure;
    case "max_unhedged_ms":
      return rt.maxExposureMs;
    default:
      return null;
  }
}

function mapRiskGroups(raw: Raw, rt: Runtime): RiskGroup[] {
  return arr<Raw>(raw.groups).map((g) => ({
    name: str(g.name),
    source: str(g.source, "§3"),
    items: arr<Raw>(g.items).map((p): RiskParam => {
      const key = str(p.key);
      const base = num(p.value);
      const equityPct = EQUITY_PCT_KEYS.has(key);
      const limit = equityPct ? (rt.equity * base) / 100 : base;
      const cur = runtimeValue(key, rt);
      const dirRaw = str(p.dir).toUpperCase();
      const dir: RiskParam["dir"] =
        key === "audit_capital_usd" ? "ref" : dirRaw === "MIN" ? "min" : dirRaw === "ZERO" ? "zero" : "max";
      return {
        key: str(p.label, key),
        limit,
        cur,
        unit: equityPct ? "USDC" : str(p.unit),
        sev: str(p.severity).replace(/_/g, " "),
        scope: str(p.scope, "GLOBAL"),
        v: str(p.version, "v1.0"),
        dir,
        pct: cur === null || !limit ? null : Math.min(999, (cur / limit) * 100),
      };
    }),
  }));
}

/* ---------------------------------- venues -------------------------------- */

function mapVenues(raw: Raw): Venue[] {
  return arr<Raw>(obj(raw).venues).map((v) => ({
    id: str(v.venue),
    kind: str(v.kind, "CEX"),
    name: str(v.name, str(v.venue)),
    short: str(v.name, str(v.venue)).toUpperCase(),
    chain: v.chain ? str(v.chain) : undefined,
    exchange: str(v.kind) === "CEX" ? str(v.name) : undefined,
    address: str(v.kind) === "DEX" ? "本地区块链账户" : undefined,
    state: str(v.state, "PAPER_READY"),
    assets: arr<Raw>(v.assets).map((a) => ({ sym: str(a.asset), qty: num(a.qty) })),
    value: num(v.usd_value),
  }));
}

/* -------------------------------- snapshot -------------------------------- */

export interface LiveSnapshot {
  patch: Partial<DataModel>;
  scannerRunning: boolean;
  live: true;
}

export async function fetchLive(): Promise<LiveSnapshot> {
  const [
    scanner,
    opps,
    exec,
    perf,
    led,
    accounts,
    risk,
    riskEventsRes,
    params,
    rules,
    chains,
    poolsRes,
    tokensRes,
    protocolsRes,
    adaptersRes,
    exchangesRes,
    health,
    pricesRaw,
    audit,
    contract,
    system,
  ] = await Promise.all([
    safe(get<Raw>("/scanner/status")),
    safe(get<Raw>("/opportunities?limit=300")),
    safe(get<Raw>("/executions")),
    safe(get<Raw>("/paper/performance")),
    safe(get<Raw>("/ledger?limit=400")),
    safe(get<Raw>("/paper/accounts")),
    safe(get<Raw>("/risk/status")),
    safe(get<Raw>("/risk/events")),
    safe(get<Raw>("/risk/params")),
    safe(get<Raw>("/risk/rules")),
    safe(get<Raw>("/chains")),
    safe(get<Raw>("/dex/pools")),
    safe(get<Raw>("/tokens")),
    safe(get<Raw>("/dex/protocols")),
    safe(get<Raw>("/adapters")),
    safe(get<Raw>("/exchanges")),
    safe(get<Raw>("/markets/data-health")),
    safe(get<Raw>("/markets/prices")),
    safe(get<Raw>("/audit/jobs")),
    safe(get<Raw>("/contract/status")),
    safe(get<Raw>("/system/status")),
  ]);

  const status = obj(scanner);
  if (!status.state && !system) throw new Error("backend unreachable");

  const oppList = arr<Raw>(obj(opps).opportunities).map(mapOpportunity);
  const execList = arr<Raw>(obj(exec).executions).map(mapExecution);
  const books = arr<Raw>(obj(health).books);
  const { prices, tickers } = readPrices(pricesRaw);
  const instruments = tickers.map((t) => mapInstrument(t, undefined));

  // 盘口摘要：把深度与盘口年龄补回行情（tickers 里只有价格）
  for (const inst of instruments) {
    const book = books.find((b) => pairOf(str(b.symbol)) === inst.symbol);
    if (book) {
      inst.depth10 = num(book.depth_ask_usd);
      inst.age = num(book.age_ms);
      inst.integrity = bool(book.valid) ? "VALID" : "INVALID";
      inst.state = bool(book.valid) ? "TRADING" : "STALE";
      inst.tradable = bool(book.valid);
    }
  }

  const perfRaw = obj(perf);
  const series = mapSeries(arr<Raw>(perfRaw.series));
  const startEquity = num(perfRaw.start_equity, 5000);
  const equity = num(perfRaw.equity, num(obj(system).equity, startEquity));
  if (series.length === 0 && equity > 0) {
    const now = new Date();
    series.push(
      { i: 0, equity: startEquity, date: new Date(now.getTime() - 86400000), peak: startEquity, dd: 0 },
      {
        i: 1,
        equity,
        date: now,
        peak: Math.max(startEquity, equity),
        dd: Math.max(0, (Math.max(startEquity, equity) - equity) / Math.max(startEquity, equity)),
      },
    );
  } else if (series.length === 1) {
    series.unshift({ i: 0, equity: startEquity, date: new Date(series[0].date.getTime() - 86400000), peak: startEquity, dd: 0 });
    series.forEach((p, i) => (p.i = i));
  }
  const last = series[series.length - 1] ?? { equity, peak: equity, dd: 0 };
  const prev = series[series.length - 2] ?? last;
  const weekAgo = series[Math.max(0, series.length - 8)] ?? prev;
  const maxDD = Math.max(0, ...series.map((p) => p.dd));
  const maxDDusd = Math.max(0, ...series.map((p) => p.peak - p.equity));

  const rt = runtimeOf(equity, obj(risk), oppList, execList, books);
  const riskGroups = mapRiskGroups(obj(params), rt);

  const chainList: ChainInfo[] = arr<Raw>(obj(chains).chains).map((c) => {
    const stage = str(c.stage);
    const stageIndex = ["DISCOVERED", "DATA_VALIDATED", "HISTORICAL_READY", "PAPER_READY", "LIVE_CANDIDATE", "LIVE_APPROVED"].indexOf(stage);
    return {
      name: str(c.name),
      id: num(c.chain_id),
      status: stage,
      stage: Math.max(0, stageIndex),
      rpc: str(c.rpc_primary),
      rpc2: str(c.rpc_secondary),
      block: num(c.head_block),
      lag: 0,
      gas: str(c.gas_model, "—"),
      pools: 0,
      quality: str(c.data_quality, "C"),
      native: str(c.native, "ETH"),
      blockTime: `${(num(c.block_time_ms) / 1000).toFixed(2)} s`,
      finality: bool(c.active) ? "本地模拟" : "未接入",
      gasModel: str(c.gas_model),
      adapter: bool(c.active) ? "demo-mkt-v1" : "—",
      enabledAt: "—",
      ws: str(c.rpc_secondary, "—"),
      archive: false,
    };
  });

  const rpcNodes: RpcNode[] = arr<Raw>(obj(chains).chains).flatMap((c) =>
    [
      { role: "PRIMARY", endpoint: str(c.rpc_primary) },
      { role: "SECONDARY", endpoint: str(c.rpc_secondary) },
    ].map((x) => ({
      chain: str(c.name),
      role: x.role,
      endpoint: x.endpoint,
      block: num(c.head_block),
      hash: `0x${num(c.head_block).toString(16).padStart(6, "0")}`,
      p95: 0,
      err: 0,
      state: "UP",
    })),
  );

  const poolList: Pool[] = arr<Raw>(obj(poolsRes).pools).map((p) => ({
    addr: `${str(p.address).slice(0, 8)}…${str(p.address).slice(-4)}`,
    protocol: poolName(str(p.pool_id)),
    pair: `${str(p.token0)}/${str(p.token1)}`,
    fee: `${(num(p.fee_bps) / 100).toFixed(2)}%`,
    chain: cap(chainOf(p.chain_id)),
    tvl: num(p.tvl_usd),
    vol24: 0,
    state: bool(p.execution_allowed) ? "ENABLED" : "OBSERVE",
    adapter: str(p.protocol_id),
    age: `${num(p.age_ms)} ms`,
  }));

  const tokenList: TokenRow[] = arr<Raw>(obj(tokensRes).tokens).map((t) => ({
    sym: str(t.symbol),
    chain: "Arbitrum",
    addr: `${str(t.address).slice(0, 8)}…${str(t.address).slice(-4)}`,
    dec: num(t.decimals, 18),
    hash: "—",
    tags: arr<string>(t.risk_tags).length ? arr<string>(t.risk_tags) : ["标准 ERC-20"],
    start: bool(t.allowed_start),
    mid: bool(t.allowed_hop),
    flash: bool(t.allowed_flash),
    state: bool(t.execution_allowed) ? "ALLOWED" : bool(t.enabled) ? "OBSERVE" : "BLOCKED",
  }));

  const protocols = arr<Raw>(obj(protocolsRes).protocols);
  const dexProtocols: DexProtocol[] = protocols.map((p) => ({
    name: poolName(str(p.protocol_id)),
    type: str(p.kind),
    pools: 1,
    chains: 1,
    state: bool(p.enabled) ? "ENABLED" : "OBSERVE",
    adapter: str(p.adapter),
  }));
  const adapters: Adapter[] = arr<Raw>(obj(adaptersRes).adapters).map((a) => ({
    protocol: poolName(str(a.protocol_id)),
    chain: "Arbitrum",
    version: str(a.version, "v1"),
    factory: str(a.protocol_id),
    router: "—",
    hash: "—",
    audit: bool(a.verified) ? "本地适配验证通过" : "未验证",
    state: bool(a.enabled) ? "ENABLED" : "BLOCKED",
  }));

  const exchangeList: Exchange[] = arr<Raw>(obj(exchangesRes).exchanges).map((e) => ({
    name: cap(str(e.name, str(e.exchange_id))),
    state: bool(e.enabled) ? "ENABLED" : "DISABLED",
    adapter: str(e.exchange_id),
    symbols: arr(e.symbols).length,
    bookAge: instruments.length ? `${Math.min(...instruments.map((i) => i.age))} ms` : "—",
    latency: "—",
    gaps: 0,
    feeSync: bool(e.trading_enabled) ? "配置声明" : "只读",
  }));

  const cexIntegrity: BookIntegrity[] = books.map((b) => ({
    exchange: cap(str(b.exchange)),
    symbol: pairOf(str(b.symbol)),
    seq: num(b.seq),
    gaps: bool(b.valid) ? 0 : 1,
    reconnect: 0,
    age: num(b.age_ms),
    latency: 0,
    lastSync: clock(b.synced_at, true),
    state: bool(b.valid) ? "VALID" : "INVALID",
  }));

  const ledgerEntries: LedgerEntry[] = arr<Raw>(obj(led).entries).map((e) => ({
    ts: clock(e.ts),
    id: shortId(str(e.entry_id), "LE"),
    account: str(e.account_id)
      .replace(":USDC:DEX_WALLET", " · DEX")
      .replace(":USDT:CEX_SPOT", " · CEX")
      .replace(":USDC:GAS_EXPENSE", " · GAS"),
    type: str(e.kind),
    memo: str(e.memo),
    amount: num(e.amount),
    balance: num(e.balance_after),
    ref: e.execution_id ? shortId(str(e.execution_id), "EX") : str(e.ref_id, "—"),
    unit: str(e.asset, "USDC"),
  }));

  const ledgerAccounts: LedgerAccount[] = arr<Raw>(obj(led).accounts).map((a) => ({
    name: str(a.label, str(a.account_id)),
    type: str(a.kind),
    balance: 0,
    note: `${str(a.venue)} · ${str(a.asset)}`,
  }));

  const flashLoans: FlashLoan[] = ledgerEntries
    .filter((e) => e.type.includes("FLASH"))
    .slice(0, 8)
    .map((e, i) => ({
      id: `FL-${i + 1}`,
      provider: "aave-v3",
      token: e.unit ?? "USDC",
      amount: Math.abs(e.amount),
      feeBps: 5,
      fee: 0,
      block: 0,
      state: e.type === "FLASH_BORROW" ? "BORROW" : "REPAY",
      tx: e.ref,
    }));

  const riskEvents: RiskEvent[] = arr<Raw>(obj(riskEventsRes).events).map((e) => ({
    id: shortId(str(e.event_id), "RE"),
    ts: clock(e.triggered_at),
    level: str(e.severity),
    scope: str(e.scope_ref, str(e.scope)),
    msg: str(e.message),
    state: e.resolved_at ? "RESOLVED" : "OPEN",
  }));

  const contractRes = obj(contract);
  const contractComponents: ContractComponent[] = arr<Raw>(contractRes.components).map((c) => ({
    name: str(c.name),
    role: str(c.role),
    addr: "未部署",
    version: "v1.0",
    state: str(c.status, "DESIGNED"),
    note: "第一阶段仅本地模拟与测试计划",
  }));
  const contractTests: ContractTest[] = arr<Raw>(contractRes.tests).map((t) => ({
    group: str(t.group),
    total: arr(t.items).length,
    pass: 0,
    note: arr<string>(t.items).join(" · "),
  }));
  const contractEvents: ContractEvent[] = arr<string>(contractRes.events).map((e) => ({
    name: e,
    n: 0,
    note: "链上未部署",
  }));

  const auditJobs: AuditJob[] = arr<Raw>(obj(audit).jobs).map((j) => ({
    id: shortId(str(j.job_id), "AJ"),
    range: "本机记录区间",
    market: str(j.dataset),
    model: str(j.model_level, "M2"),
    coverage: 0,
    gaps: 0,
    samples: 0,
    verdict: (str(j.verdict, "WATCH") as AuditJob["verdict"]),
    note: str(j.reason),
  }));

  const services: Service[] = [
    {
      name: "arb-backend",
      role: "Axum API + WS + 扫描器 + Paper 引擎",
      state: "UP",
      uptime: msHuman(num(status.uptime_ms)),
      p95: `${num(obj(system).stats && obj(obj(system).stats).last_scan_ms)} ms`,
      note: str(obj(system).db, ""),
    },
    { name: "feed", role: `行情来源：${str(obj(system).feed, "demo")}`, state: "UP", uptime: "—", p95: "—", note: "本地合成市场 / 可切换实时公共行情" },
    { name: "sqlite", role: "本机嵌入式数据库", state: "UP", uptime: "—", p95: "—", note: str(obj(system).db, "") },
    { name: "arb-audit", role: "历史审计 CLI", state: "IDLE", uptime: "—", p95: "—", note: "arb-backend audit" },
  ];

  const healthRows = [
    { key: "RPC", value: "UP", note: "主备节点配置就绪 · 本地模拟链路" },
    { key: "CEX WS", value: books.length && books.every((b) => bool(b.valid)) ? "UP" : books.length ? "DEGRADED" : "—", note: `Binance 公共行情 · ${books.length} 个盘口` },
    { key: "数据库", value: "UP", note: "SQLite WAL · 本地单文件" },
    { key: "历史数据", value: "LIMITED", note: "第一阶段未导入历史 L2 盘口" },
  ];

  const running = str(status.state) === "RUNNING";
  const byStrategy: Record<string, { candidates: number; simulated: number; passed: number }> = {
    dex_triple: { candidates: 0, simulated: 0, passed: 0 },
    dex_cross: { candidates: 0, simulated: 0, passed: 0 },
    cex_dex: { candidates: 0, simulated: 0, passed: 0 },
  };
  for (const o of oppList) {
    const bucket = byStrategy[o.strategy];
    if (!bucket) continue;
    bucket.candidates += 1;
    bucket.simulated += 1;
    if (o.status === "ACTIONABLE") bucket.passed += 1;
  }

  const patch: Partial<DataModel> = {
    opps: oppList,
    executions: execList,
    instruments,
    cexIntegrity,
    chains: chainList,
    rpcNodes,
    pools: poolList,
    tokens: tokenList,
    adapters,
    dexProtocols,
    exchanges: exchangeList,
    ledger: ledgerEntries,
    ledgerAccounts,
    flashLoans,
    riskGroups,
    riskParams: riskGroups.flatMap((g) => g.items),
    riskEvents,
    venues: mapVenues(obj(accounts)),
    contractComponents,
    contractTests,
    contractEvents,
    auditJobs,
    services,
    health: healthRows,
    circuitRules: arr<Raw>(obj(rules).circuit_rules).map<CircuitRule>((r) => ({
      trigger: str(r.trigger),
      scope: str(r.scope),
      recover: str(r.recovery),
      last: "—",
    })),
    series,
    equity,
    startEquity,
    maxDD,
    maxDDusd,
    todayPnl: last.equity - prev.equity,
    weekPnl: last.equity - weekAgo.equity,
    px: { ...prices, ETH: prices.ETH ?? prices.WETH, BTC: prices.BTC ?? prices.WBTC, USDC: prices.USDC ?? 1, USDT: prices.USDT ?? 1 },
    routeSpace: {
      tokens: 5,
      pools: num(status.pools),
      hops: 4,
      candidates: oppList.length,
      pruned: Math.max(0, oppList.length - num(status.actionable)),
      simulated: oppList.length,
      passed: num(status.actionable),
      byStrategy,
    },
    scanner: {
      state: str(status.state, "STOPPED"),
      started: clock(status.started_at),
      uptime: msHuman(num(status.uptime_ms)),
      quotes: num(status.quotes),
      detected: num(status.detected),
      actionable: num(status.actionable),
      rejected: num(status.rejected),
      watch: num(status.watch),
      expired: num(status.expired),
      executed: execList.length,
      chains: num(status.chains, chainList.length),
      protocols: num(status.protocols, dexProtocols.length),
      pools: num(status.pools, poolList.length),
      lastScanMs: num(status.last_scan_ms),
      intervalMs: num(status.scan_interval_ms),
    },
    riskStatus: {
      equity,
      startEquity,
      accountDailyLoss: num(obj(risk).account_daily_loss),
      realizedPnl: num(obj(risk).realized_pnl),
      maxDrawdown: num(obj(risk).max_drawdown),
      globalPaused: bool(obj(obj(obj(risk).controls).global).paused),
      pausedScopes: arr<Raw>(obj(risk).active_pauses).map((p) => ({
        scope: str(arr(p)[0]),
        ref: str(arr(p)[1]),
        reason: str(arr(p)[2]),
      })),
      paramVersion: str(obj(params).version, "risk-v1.0"),
    },
    meta: {
      online: true,
      lastSync: Date.now(),
      lastError: null,
      apiBase: BASE,
      feed: str(obj(system).feed, "demo"),
      mode: str(obj(system).mode, "PAPER"),
      version: str(obj(system).version, "0.1.0"),
      db: str(obj(system).db, ""),
      liveTrading: bool(obj(system).live_trading_enabled),
    },
  };

  return { patch, scannerRunning: running, live: true };
}

/* ------------------------------- write actions ---------------------------- */

export async function scannerAction(action: "start" | "stop"): Promise<void> {
  const res = await fetch(`${BASE}/scanner/${action}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: "{}",
  });
  if (!res.ok) throw new Error(`scanner ${action} → HTTP ${res.status}`);
}

export async function riskPause(paused: boolean, reason = ""): Promise<void> {
  const res = await fetch(`${BASE}/risk/${paused ? "pause" : "resume"}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ scope: "GLOBAL", reason: reason || (paused ? "界面手动暂停" : "界面手动恢复") }),
  });
  if (!res.ok) {
    let message = `risk ${paused ? "pause" : "resume"} → HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      /* ignore */
    }
    throw new Error(message);
  }
}

export async function executeOpportunity(id: string): Promise<void> {
  const res = await fetch(`${BASE}/opportunities/${encodeURIComponent(id)}/execute`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: "{}",
  });
  if (!res.ok) {
    let message = `execute → HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      /* ignore */
    }
    throw new Error(message);
  }
}

export async function resetPaper(): Promise<void> {
  const res = await fetch(`${BASE}/paper/reset`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: "{}",
  });
  if (!res.ok) throw new Error(`paper reset → HTTP ${res.status}`);
}

export async function runAudit(dataset = "paper-sessions"): Promise<{ verdict: string; reason: string }> {
  const res = await fetch(`${BASE}/audit/jobs`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ dataset }),
  });
  if (!res.ok) throw new Error(`audit → HTTP ${res.status}`);
  const body = (await res.json()) as { report?: { verdict: string; reason: string } };
  return body.report ?? { verdict: "WATCH", reason: "已提交" };
}

export type { DexProtocol };

/* --------------------------- DEX route search ---------------------------- */
/*
 * GET /api/routes、/api/routes/{id}、/api/dex/graph、/api/scanner/engine
 * 与 POST /api/simulations 的映射。金额字段是 Decimal 字符串，统一转 number。
 */

const strList = (v: unknown): string[] => arr<unknown>(v).map((x) => str(x));

function mapGasEstimate(raw: unknown): GasEstimate | null {
  if (!raw || typeof raw !== "object") return null;
  const g = obj(raw);
  return {
    units: num(g.units),
    gas_price_gwei: num(g.gas_price_gwei),
    native_price_usd: num(g.native_price_usd),
    usd: num(g.usd),
    components: arr<Raw>(g.components).map((c) => ({
      key: str(c.key),
      label: str(c.label, str(c.key)),
      units: num(c.units),
    })),
    block_number: num(g.block_number),
    estimated_at: str(g.estimated_at),
    note: str(g.note),
  };
}

function mapInputPoint(raw: Raw): InputPoint {
  return {
    amount_in: num(raw.amount_in),
    amount_in_usd: num(raw.amount_in_usd),
    amount_out: num(raw.amount_out),
    gross_usd: num(raw.gross_usd),
    cost_usd: num(raw.cost_usd),
    net_usd: num(raw.net_usd),
    net_bps: num(raw.net_bps),
    gas_usd: num(raw.gas_usd),
    gas_share_pct: num(raw.gas_share_pct),
    price_impact_bps: num(raw.price_impact_bps),
    source: str(raw.source),
  };
}

function mapOptimization(raw: unknown): InputOptimization | null {
  if (!raw || typeof raw !== "object") return null;
  const o = obj(raw);
  return {
    points: arr<Raw>(o.points).map(mapInputPoint),
    best: o.best && typeof o.best === "object" ? mapInputPoint(obj(o.best)) : null,
    min_profitable_amount: o.min_profitable_amount == null ? null : num(o.min_profitable_amount),
    max_profitable_amount: o.max_profitable_amount == null ? null : num(o.max_profitable_amount),
    max_executable_amount: num(o.max_executable_amount),
    grid_points: num(o.grid_points),
    refine_iters: num(o.refine_iters),
    truncated: bool(o.truncated),
    notes: strList(o.notes),
  };
}

function mapSimulation(raw: unknown): SimulationOutcome | null {
  if (!raw || typeof raw !== "object") return null;
  const s = obj(raw);
  return {
    simulation_id: str(s.simulation_id),
    level: str(s.level, "M1") as SimulationOutcome["level"],
    status: str(s.status, "UNAVAILABLE") as SimulationStatus,
    quoted_out: num(s.quoted_out),
    simulated_out: num(s.simulated_out),
    diff_bps: num(s.diff_bps),
    net_usd: s.net_usd == null ? null : num(s.net_usd),
    gas_estimate: mapGasEstimate(s.gas_estimate),
    state_block: num(s.state_block),
    notes: strList(s.notes),
    created_at: str(s.created_at),
  };
}

function mapViolation(raw: Raw): RiskViolationRecord {
  return {
    key: str(raw.key),
    label: str(raw.label, str(raw.key)),
    severity: str(raw.severity, "WARNING"),
    limit: num(raw.limit),
    actual: num(raw.actual),
    unit: str(raw.unit),
    message: str(raw.message),
  };
}

function mapRouteLeg(raw: Raw): RouteLegRecord {
  return {
    idx: num(raw.idx),
    pool_id: str(raw.pool_id),
    protocol_id: str(raw.protocol_id),
    amm: str(raw.amm),
    token_in: str(raw.token_in),
    token_out: str(raw.token_out),
    amount_in: num(raw.amount_in),
    amount_out: num(raw.amount_out),
    fee_bps: num(raw.fee_bps),
    price_impact_bps: num(raw.price_impact_bps),
    gas_units: num(raw.gas_units),
    pool_tvl_usd: num(raw.pool_tvl_usd),
    block_number: num(raw.block_number),
    state_age_ms: num(raw.state_age_ms),
  };
}

function mapRouteScanRecord(raw: Raw): RouteScanRecord {
  const fp = obj(raw.fingerprints);
  return {
    route_id: str(raw.route_id),
    chain_id: num(raw.chain_id),
    start_token: str(raw.start_token),
    route_type: str(raw.route_type, "CROSS_POOL") as RouteType,
    fingerprints: {
      directed: str(fp.directed),
      undirected: str(fp.undirected),
      pool_sequence: str(fp.pool_sequence),
      token_sequence: str(fp.token_sequence),
      start_asset: str(fp.start_asset),
    },
    token_path: strList(raw.token_path),
    pool_path: strList(raw.pool_path),
    protocols: strList(raw.protocols),
    amms: strList(raw.amms),
    hops: num(raw.hops),
    legs: arr<Raw>(raw.legs).map(mapRouteLeg),
    amount_in: num(raw.amount_in),
    amount_out: num(raw.amount_out),
    notional_usd: num(raw.notional_usd),
    gross_usd: num(raw.gross_usd),
    total_cost_usd: num(raw.total_cost_usd),
    net_usd: num(raw.net_usd),
    net_bps: num(raw.net_bps),
    embedded_fee_usd: num(raw.embedded_fee_usd),
    gas: mapGasEstimate(raw.gas),
    flash_funded: bool(raw.flash_funded),
    optimization: mapOptimization(raw.optimization),
    simulation: mapSimulation(raw.simulation),
    stress: mapSimulation(raw.stress),
    lifecycle: str(raw.lifecycle, "DISCOVERED") as RouteLifecycle,
    rejections: arr<Raw>(raw.rejections).map(mapViolation),
    block_number: num(raw.block_number),
    quote_age_ms: num(raw.quote_age_ms),
    detected_at: str(raw.detected_at),
    expires_at: str(raw.expires_at),
    model_level: str(raw.model_level, "M1") as RouteScanRecord["model_level"],
    opportunity_id: raw.opportunity_id == null ? null : str(raw.opportunity_id),
    capacity_usd: num(raw.capacity_usd),
    size_quotes: arr<Raw>(raw.size_quotes).map((q) => ({
      notional_usd: num(q.notional_usd),
      amount_in: num(q.amount_in),
      amount_out: num(q.amount_out),
      gross_usd: num(q.gross_usd),
      total_cost_usd: num(q.total_cost_usd),
      net_usd: num(q.net_usd),
      net_bps: num(q.net_bps),
    })),
    source: str(raw.source, "full"),
  };
}

function mapGraphStats(raw: unknown): GraphStats {
  const g = obj(raw);
  return {
    chain_id: num(g.chain_id),
    tokens: num(g.tokens),
    approved_tokens: num(g.approved_tokens),
    pools: num(g.pools),
    eligible_pools: num(g.eligible_pools),
    edges: num(g.edges),
    rejected_pools: num(g.rejected_pools),
    built_at: typeof g.built_at === "string" ? g.built_at : null,
    signature: str(g.signature),
  };
}

function mapScanRunStats(raw: unknown): ScanRunStats {
  const s = obj(raw);
  return {
    run_id: str(s.run_id),
    started_at: typeof s.started_at === "string" ? s.started_at : null,
    elapsed_ms: num(s.elapsed_ms),
    full_rescan: bool(s.full_rescan, true),
    reason: str(s.reason),
    pools: num(s.pools),
    eligible_pools: num(s.eligible_pools),
    tokens: num(s.tokens),
    approved_tokens: num(s.approved_tokens),
    edges: num(s.edges),
    candidates: num(s.candidates),
    requoted: num(s.requoted),
    reused: num(s.reused),
    visits: num(s.visits),
    pruned_liquidity: num(s.pruned_liquidity),
    pruned_risk: num(s.pruned_risk),
    truncated: bool(s.truncated),
    rejected_by_gate: num(s.rejected_by_gate),
    profitable: num(s.profitable),
    base_tokens: strList(s.base_tokens),
    min_hops: num(s.min_hops, 2),
    max_hops: num(s.max_hops, 5),
    model_level: str(s.model_level, "M1") as ScanRunStats["model_level"],
    notes: strList(s.notes),
  };
}

function mapCacheStats(raw: unknown): RouteCacheStats {
  const c = obj(raw);
  return {
    routes: num(c.routes),
    topology_signature: str(c.topology_signature),
    empty: typeof c.empty === "boolean" ? c.empty : undefined,
  };
}

function mapEngineParams(raw: unknown): DexEngineParams {
  const p = obj(raw);
  const stress = obj(p.stress);
  return {
    model_note: str(p.model_note),
    min_hops: num(p.min_hops, 2),
    max_hops: num(p.max_hops, 5),
    max_candidates: num(p.max_candidates),
    max_pools: num(p.max_pools),
    max_visits: num(p.max_visits),
    max_elapsed_ms: num(p.max_elapsed_ms),
    min_input_usd: num(p.min_input_usd),
    max_input_usd: num(p.max_input_usd),
    min_pool_tvl_usd: num(p.min_pool_tvl_usd),
    max_pool_state_age_ms: num(p.max_pool_state_age_ms),
    max_pool_share_pct: num(p.max_pool_share_pct),
    grid_points: num(p.grid_points),
    refine_iters: num(p.refine_iters),
    base_symbols: strList(p.base_symbols),
    notional_ladder: arr<unknown>(p.notional_ladder).map((x) => num(x)),
    flash_fee_bps: num(p.flash_fee_bps),
    priority_fee_usd: num(p.priority_fee_usd),
    delay_reserve_bps: num(p.delay_reserve_bps),
    mev_reserve_bps: num(p.mev_reserve_bps),
    profit_discount_pct: num(p.profit_discount_pct),
    model_mismatch_max_bps: num(p.model_mismatch_max_bps),
    requote_age_ms: num(p.requote_age_ms),
    full_rescan_blocks: num(p.full_rescan_blocks),
    ttl_sec: num(p.ttl_sec),
    gas_price_gwei: num(p.gas_price_gwei),
    native_price_usd: num(p.native_price_usd),
    stress: {
      adverse_bps: num(stress.adverse_bps),
      gas_increase_pct: num(stress.gas_increase_pct),
      flash_fee_increase_pct: num(stress.flash_fee_increase_pct),
    },
  };
}

export interface RouteQueryParams {
  route_type?: RouteType;
  lifecycle?: RouteLifecycle;
  start_token?: string;
  min_net?: number;
  min_hops?: number;
  max_hops?: number;
  limit?: number;
}

/** GET /api/routes —— 路线集合、图层统计与本轮扫描统计。 */
export async function routes(params: RouteQueryParams = {}): Promise<RouteScanResponse> {
  const qs = new URLSearchParams();
  if (params.route_type) qs.set("route_type", params.route_type);
  if (params.lifecycle) qs.set("lifecycle", params.lifecycle);
  if (params.start_token) qs.set("start_token", params.start_token);
  if (params.min_net !== undefined) qs.set("min_net", String(params.min_net));
  if (params.min_hops !== undefined) qs.set("min_hops", String(params.min_hops));
  if (params.max_hops !== undefined) qs.set("max_hops", String(params.max_hops));
  if (params.limit !== undefined) qs.set("limit", String(params.limit));
  const suffix = qs.toString() ? `?${qs.toString()}` : "";
  const raw = await get<Raw>(`/routes${suffix}`);
  return {
    total: num(raw.total),
    routes: arr<Raw>(raw.routes).map(mapRouteScanRecord),
    graph: mapGraphStats(raw.graph),
    scan: mapScanRunStats(raw.scan),
  };
}

/** GET /api/routes/{id} —— 单条路线的完整审计记录（含逐腿与优化曲线）。 */
export async function routeDetail(routeId: string): Promise<RouteScanRecord> {
  const raw = await get<Raw>(`/routes/${encodeURIComponent(routeId)}`);
  return mapRouteScanRecord(raw);
}

/** GET /api/dex/graph —— 图层统计与增量缓存状态。 */
export async function dexGraph(): Promise<DexGraphResponse> {
  const raw = await get<Raw>("/dex/graph");
  return {
    graph: mapGraphStats(raw.graph),
    scan: mapScanRunStats(raw.scan),
    cache: mapCacheStats(raw.cache),
  };
}

/** GET /api/scanner/engine —— 引擎参数、图层与最近一轮扫描统计。 */
export async function scannerEngine(): Promise<DexEngineResponse> {
  const raw = await get<Raw>("/scanner/engine");
  return {
    params: mapEngineParams(raw.params),
    scan: mapScanRunStats(raw.scan),
    graph: mapGraphStats(raw.graph),
    cache: mapCacheStats(raw.cache),
  };
}

async function postJson<T>(path: string, body: unknown): Promise<T> {  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    let message = `${path} → HTTP ${res.status}`;
    try {
      const payload = obj(await res.json());
      const err = payload.error;
      if (typeof err === "string") message = err;
      else if (err && typeof err === "object" && typeof obj(err).message === "string") {
        message = str(obj(err).message);
      }
    } catch {
      /* 非 JSON 错误响应 */
    }
    throw new Error(message);
  }
  return (await res.json()) as T;
}

/** POST /api/simulations —— 对已记录路线重新运行 M1 / M3（M2 返回 UNAVAILABLE）。 */
export async function runSimulation(
  routeId: string,
  level: SimulationLevel,
): Promise<SimulationOutcome> {
  const raw = await postJson<Raw>("/simulations", { route_id: routeId, level });
  const simulation = mapSimulation(raw.simulation);
  if (!simulation) throw new Error("模拟响应缺少 simulation 字段");
  return simulation;
}

/* ------------------------------ search config ----------------------------- */

async function sendJson<T>(method: "PUT" | "POST", path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    let message = `${path} → HTTP ${res.status}`;
    try {
      const payload = obj(await res.json());
      if (typeof payload.error === "string") message = payload.error;
      if (Array.isArray(payload.errors)) message = payload.errors.map(String).join("；");
    } catch {
      /* ignore */
    }
    throw new Error(message);
  }
  return (await res.json()) as T;
}

export async function fetchPrices(): Promise<PricesResponse> {
  return get<PricesResponse>("/system/prices");
}

export async function fetchSearchConfig(): Promise<SearchConfigResponse> {
  return get<SearchConfigResponse>("/search/config");
}

export async function saveSearchConfig(body: {
  params?: SearchParamsConfig;
  tokens?: SearchTokenEntry[];
}): Promise<{ ok: boolean; token_reload_required?: boolean }> {
  return sendJson("PUT", "/search/config", body);
}

export async function updateSearchToken(body: {
  action: "whitelist" | "blacklist" | "remove";
  symbol: string;
  address?: string;
  decimals?: number;
}): Promise<{ ok: boolean; tokens: SearchTokenEntry[] }> {
  return sendJson("POST", "/search/tokens", body);
}

/* ------------------------------- discovery -------------------------------- */

export async function discoveryStatus(): Promise<DiscoveryStatus> {
  return get<DiscoveryStatus>("/discovery/status");
}

export async function discoveryScan(): Promise<DiscoveryStatus> {
  return postJson<DiscoveryStatus>("/discovery/scan", {});
}

/* ------------------------------- sources --------------------------------- */

export async function fetchSources(): Promise<SourceConfig> {
  return get<SourceConfig>("/sources");
}

export async function saveSources(body: SourceConfig): Promise<SourceConfig> {
  const res = await fetch(`${BASE}/sources`, {
    method: "PUT",
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    let message = `sources HTTP ${res.status}`;
    try {
      const p = obj(await res.json());
      if (typeof p.error === "string") message = p.error;
    } catch {
      /* ignore */
    }
    throw new Error(message);
  }
  return (await res.json()) as SourceConfig;
}

export async function applySourcePreset(
  mode: "free" | "boost",
): Promise<SourcePresetResponse> {
  return postJson<SourcePresetResponse>("/sources/preset", { mode });
}
