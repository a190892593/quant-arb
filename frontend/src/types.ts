/** domain model shared with the Rust backend contracts (TDD §6.1 / §10) */

export type StrategyId = "dex_triple" | "dex_cross" | "cex_dex";
export type OppStatus = "ACTIONABLE" | "WATCH" | "REJECTED" | "EXPIRED" | "EXECUTED";
/** data_quality: A/B/C 为可执行等级，D 与 INVALID 由后端用于拒绝 */
export type Quality = "A" | "B" | "C" | "D" | "INVALID";
export type ChainKey = "arbitrum" | "base" | "ethereum" | "polygon";

export interface OpportunityCosts {
  dexFee: number;
  cexFee: number;
  gas: number;
  priority: number;
  flash: number;
  slippage: number;
  hedge: number;
  rebalance: number;
}

export interface OpportunityFacts {
  hops: number;
  impactBps: number;
  rpcOk: boolean;
  tokenOk: boolean;
  providerOk: boolean;
  inventoryOk: boolean;
  competition: number;
  depthUsage?: number;
  bookAgeMs?: number;
}

export interface Leg {
  n: number;
  from: string;
  to: string;
  pool: string;
  pair?: string;
  fee: string;
  inQty: string;
  outQty: string;
  minOut: string;
  impact: string;
}

export interface Opportunity {
  id: string;
  strategy: StrategyId;
  chain: ChainKey;
  cex?: string;
  symbol?: string;
  venue: string;
  pair: string;
  notional: number;
  theo: number;
  gross: number;
  costs: OpportunityCosts;
  stressNet: number;
  riskScore: number;
  quality: Quality;
  status: OppStatus;
  ageMs: number;
  model: "M0" | "M1" | "M2" | "M3";
  block: number;
  blockDelta: number;
  quoteAgeMs: number;
  facts: OpportunityFacts;
  reasons?: string[];
  legs?: Leg[];
  executionId?: string;
  /* live API extras (optional so the demo dataset stays valid) */
  rawId?: string;
  direction?: string | null;
  hedgeOrder?: string | null;
  expiresAt?: string;
  capacity?: number;
  detectedAt?: string;
  /* derived by the dataset builder */
  costTotal: number;
  net: number;
  minProfit: number;
  netBps: number;
  theoBps: number;
  gasRatio: number;
}

export interface EquityPoint {
  i: number;
  equity: number;
  date: Date;
  peak: number;
  dd: number;
}

export interface Instrument {
  symbol: string;
  base: string;
  quote: string;
  exchange: string;
  state: string;
  tradable: boolean;
  bid: number;
  ask: number;
  spread: number;
  depth10: number;
  age: number;
  integrity: string;
  seq: number;
  tick: number;
  step: number;
  minNotional: number;
  maker: number;
  taker: number;
  pricePrec: number;
  qtyPrec: number;
  depthScale: number;
  recvTs: string;
}

export interface ChainInfo {
  name: string;
  id: number;
  status: string;
  stage: number;
  rpc: string;
  rpc2: string;
  block: number;
  lag: number;
  gas: string;
  pools: number;
  quality: string;
  native: string;
  blockTime: string;
  finality: string;
  gasModel: string;
  adapter: string;
  enabledAt: string;
  ws: string;
  archive: boolean;
}

export interface RiskParam {
  key: string;
  limit: number;
  cur: number | null;
  unit: string;
  sev: string;
  scope: string;
  v: string;
  dir: "max" | "min" | "zero" | "ref";
  pct: number | null;
}

export interface StrategyMeta {
  label: string;
  short: string;
  color: string;
}

export interface ScannerState {
  state: string;
  started: string;
  uptime: string;
  quotes: number;
  detected: number;
  actionable: number;
  rejected: number;
  watch: number;
  expired: number;
  executed: number;
  chains: number;
  protocols: number;
  pools: number;
  lastScanMs?: number;
  intervalMs?: number;
}

/** live backend connection state (not part of the demo dataset) */
export interface ConnectionMeta {
  online: boolean;
  lastSync: number | null;
  lastError: string | null;
  apiBase: string;
  feed: string;
  mode: string;
  version: string;
  db: string;
  liveTrading: boolean;
}


export interface Pool {
  addr: string;
  protocol: string;
  pair: string;
  fee: string;
  chain: string;
  tvl: number;
  vol24: number;
  state: string;
  adapter: string;
  age: string;
}

export interface RouteSpace {
  tokens: number;
  pools: number;
  hops: number;
  candidates: number;
  pruned: number;
  simulated: number;
  passed: number;
  byStrategy: Record<string, { candidates: number; simulated: number; passed: number }>;
}
export interface DataModel {
  px: Record<string, number>;
  equity: number;
  startEquity: number;
  series: EquityPoint[];
  maxDD: number;
  maxDDusd: number;
  todayPnl: number;
  weekPnl: number;
  opps: Opportunity[];
  STRATEGY: Record<StrategyId, StrategyMeta>;
  CHAIN: Record<ChainKey, { label: string; id: number }>;
  instruments: Instrument[];
  pools: Pool[];
  routeSpace: RouteSpace;
  chains: ChainInfo[];
  riskParams: RiskParam[];
  scanner: ScannerState;
  health: { key: string; value: string; note: string }[];
  venues: Venue[];
  executions: Execution[];
  EXEC_STATES: string[];
  ledger: LedgerEntry[];
  LEDGER_TYPES: string[];
  riskEvents: RiskEvent[];
  scopes: ScopeGroup[];
  auditJobs: AuditJob[];
  services: Service[];
  dexProtocols: DexProtocol[];
  exchanges: Exchange[];
  tokens: TokenRow[];
  adapters: Adapter[];
  rpcNodes: RpcNode[];
  cexIntegrity: BookIntegrity[];
  archiveCoverage: ArchiveRow[];
  clockSkew: SkewRow[];
  dataGaps: GapRow[];
  contractComponents: ContractComponent[];
  contractTests: ContractTest[];
  contractEvents: ContractEvent[];
  riskGroups: RiskGroup[];
  circuitRules: CircuitRule[];
  ledgerAccounts: LedgerAccount[];
  flashLoans: FlashLoan[];
  strategyConfig: Record<string, StrategyConfig>;
  /** live connection metadata, filled by the API layer */
  meta?: ConnectionMeta;
  /** risk status snapshot used by the shell (optional for demo data) */
  riskStatus?: {
    equity: number;
    startEquity: number;
    accountDailyLoss: number;
    realizedPnl: number;
    maxDrawdown: number;
    globalPaused: boolean;
    pausedScopes: { scope: string; ref: string; reason: string }[];
    paramVersion: string;
  };
}

/* ---------- secondary collections used by the remaining views ---------- */

export interface Venue {
  id: string;
  kind: string;
  name: string;
  short: string;
  chain?: string;
  exchange?: string;
  address?: string;
  state: string;
  assets: { sym: string; qty: number }[];
  value: number;
}

export interface ExecutionLeg {
  side: "DEX" | "CEX";
  venue: string;
  state: string;
  submitted: string;
  filled: string;
  qty: string;
  price: string;
  fee: string;
  slip: string;
}

export interface Execution {
  id: string;
  oppId: string;
  strategy: StrategyId;
  chain: ChainKey;
  cex?: string;
  symbol?: string;
  pair: string;
  notional: number;
  state: string;
  order: string;
  startedAt: string;
  duration: number;
  exposure: number;
  exposureMs: number;
  netPnl: number;
  statusNote: string;
  legs: ExecutionLeg[];
  revert?: string;
  ledger?: { account: string; delta: string }[];
  exposureSeries?: number[];
}

export interface LedgerEntry {
  ts: string;
  id: string;
  account: string;
  type: string;
  memo: string;
  amount: number;
  balance: number;
  ref: string;
  unit?: string;
}

export interface RiskEvent {
  id: string;
  ts: string;
  level: string;
  scope: string;
  msg: string;
  state: string;
}

export interface ScopeItem {
  id: string;
  label: string;
  on: boolean;
  note?: string;
}

export interface ScopeGroup {
  group: string;
  items: ScopeItem[];
}

export interface AuditJob {
  id: string;
  range: string;
  market: string;
  model: string;
  coverage: number;
  gaps: number;
  samples: number;
  verdict: "PASS" | "WATCH" | "REJECT" | "PAPER";
  note: string;
}

export interface Service {
  name: string;
  role: string;
  state: string;
  uptime: string;
  p95: string;
  note: string;
}

export interface DexProtocol {
  name: string;
  type: string;
  pools: number;
  chains: number;
  state: string;
  adapter: string;
}

export interface Exchange {
  name: string;
  state: string;
  adapter: string;
  symbols: number;
  bookAge: string;
  latency: string;
  gaps: number;
  feeSync: string;
}

export interface TokenRow {
  sym: string;
  chain: string;
  addr: string;
  dec: number;
  hash: string;
  tags: string[];
  start: boolean;
  mid: boolean;
  flash: boolean;
  state: string;
}

export interface Adapter {
  protocol: string;
  chain: string;
  version: string;
  factory: string;
  router: string;
  hash: string;
  audit: string;
  state: string;
}

export interface RpcNode {
  chain: string;
  role: string;
  endpoint: string;
  block: number;
  hash: string;
  p95: number;
  err: number;
  state: string;
}

export interface BookIntegrity {
  exchange: string;
  symbol: string;
  seq: number;
  gaps: number;
  reconnect: number;
  age: number;
  latency: number;
  lastSync: string;
  state: string;
}

export interface ArchiveRow {
  chain: string;
  from: string;
  to: string;
  coverage: number;
  readable: boolean;
  note: string;
}

export interface SkewRow {
  pair: string;
  skew: number;
  limit: number;
}

export interface GapRow {
  at: string;
  dur: string;
  source: string;
  impact: string;
}

export interface ContractComponent {
  name: string;
  role: string;
  addr: string;
  version: string;
  state: string;
  note: string;
}

export interface ContractTest {
  group: string;
  total: number;
  pass: number;
  note: string;
}

export interface ContractEvent {
  name: string;
  n: number;
  note: string;
}

export interface RiskGroup {
  name: string;
  source: string;
  items: RiskParam[];
}

export interface CircuitRule {
  trigger: string;
  scope: string;
  recover: string;
  last: string;
}

export interface LedgerAccount {
  name: string;
  type: string;
  balance: number;
  note: string;
}

export interface FlashLoan {
  id: string;
  provider: string;
  token: string;
  amount: number;
  feeBps: number;
  fee: number;
  block: number;
  state: string;
  tx: string;
}

export interface StrategyParamItem {
  key: string;
  label: string;
  value: number;
  unit: string;
  min: number;
  max: number;
  step: number;
  global: number;
}

export interface StrategyGroup {
  name: string;
  note?: string;
  items: StrategyParamItem[];
}

export interface StrategyToggle {
  key: string;
  label: string;
  on: boolean;
  note?: string;
  locked: boolean;
}

export interface StrategyConfig {
  id: StrategyId;
  name: string;
  family: string;
  thesis: string;
  mode: string;
  enabled: boolean;
  version: string;
  kpi: { label: string; value: string; note: string; tone?: string }[];
  groups: StrategyGroup[];
  toggles: StrategyToggle[];
  history: { at: string; by: string; change: string }[];
}