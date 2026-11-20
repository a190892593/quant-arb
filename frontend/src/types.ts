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
  /** M2（REVM 分叉）复核结果 */
  m2?: {
    passed: boolean;
    block: number;
    amountIn: number;
    m1Out: number;
    m2Out: number;
    diffBps: number;
    gasUsed: number;
    elapsedMs: number;
    detail: string;
  };
  /** 风控违规/告警明细（含 min_profit 缺口，用于 near-miss 展示） */
  rejections?: {
    key: string;
    label: string;
    severity: string;
    limit: number;
    actual: number;
    unit: string;
    message: string;
  }[];
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

/* ---------- DEX 多池多跳扫描（PRD §14.3 / §14.4） ---------- */
/*
 * 后端 /api/routes 的审计型记录。Decimal 以字符串传输，
 * api.ts 在映射时统一转为 number（仅用于展示，不做二次金额计算）。
 */

export type RouteType = "CROSS_POOL" | "TRIANGLE" | "MULTI_HOP";
export type RouteLifecycle =
  | "DISCOVERED"
  | "QUOTED"
  | "PROFITABLE_ESTIMATE"
  | "SIMULATED"
  | "VERIFIED"
  | "REJECTED"
  | "EXPIRED"
  | "FAILED";
export type SimulationStatus = "PASSED" | "FAILED" | "UNAVAILABLE" | "MODEL_MISMATCH";
export type ModelLevel = "M0" | "M1" | "M2" | "M3";
export type SimulationLevel = "M1" | "M3" | "M2";

export interface RouteFingerprints {
  directed: string;
  undirected: string;
  pool_sequence: string;
  token_sequence: string;
  start_asset: string;
}

export interface GasComponent {
  key: string;
  label: string;
  units: number;
}

export interface GasEstimate {
  units: number;
  gas_price_gwei: number;
  native_price_usd: number;
  usd: number;
  components: GasComponent[];
  block_number: number;
  estimated_at: string;
  note: string;
}

export interface InputPoint {
  amount_in: number;
  amount_in_usd: number;
  amount_out: number;
  gross_usd: number;
  cost_usd: number;
  net_usd: number;
  net_bps: number;
  gas_usd: number;
  gas_share_pct: number;
  price_impact_bps: number;
  source: string;
}

export interface InputOptimization {
  points: InputPoint[];
  best: InputPoint | null;
  min_profitable_amount: number | null;
  max_profitable_amount: number | null;
  max_executable_amount: number;
  grid_points: number;
  refine_iters: number;
  truncated: boolean;
  notes: string[];
}

export interface RouteLegRecord {
  idx: number;
  pool_id: string;
  protocol_id: string;
  amm: string;
  token_in: string;
  token_out: string;
  amount_in: number;
  amount_out: number;
  fee_bps: number;
  price_impact_bps: number;
  gas_units: number;
  pool_tvl_usd: number;
  block_number: number;
  state_age_ms: number;
}

export interface SimulationOutcome {
  simulation_id: string;
  level: ModelLevel;
  status: SimulationStatus;
  quoted_out: number;
  simulated_out: number;
  diff_bps: number;
  net_usd: number | null;
  gas_estimate: GasEstimate | null;
  state_block: number;
  notes: string[];
  created_at: string;
}

export interface RiskViolationRecord {
  key: string;
  label: string;
  severity: string;
  limit: number;
  actual: number;
  unit: string;
  message: string;
}

export interface SizeQuote {
  notional_usd: number;
  amount_in: number;
  amount_out: number;
  gross_usd: number;
  total_cost_usd: number;
  net_usd: number;
  net_bps: number;
}

export interface RouteScanRecord {  route_id: string;
  chain_id: number;
  start_token: string;
  route_type: RouteType;
  fingerprints: RouteFingerprints;
  token_path: string[];
  pool_path: string[];
  protocols: string[];
  amms: string[];
  hops: number;
  legs: RouteLegRecord[];
  amount_in: number;
  amount_out: number;
  notional_usd: number;
  gross_usd: number;
  total_cost_usd: number;
  net_usd: number;
  net_bps: number;
  embedded_fee_usd: number;
  gas: GasEstimate | null;
  flash_funded: boolean;
  optimization: InputOptimization | null;
  simulation: SimulationOutcome | null;
  stress: SimulationOutcome | null;
  lifecycle: RouteLifecycle;
  rejections: RiskViolationRecord[];
  block_number: number;
  quote_age_ms: number;
  detected_at: string;
  expires_at: string;
  model_level: ModelLevel;
  opportunity_id: string | null;
  capacity_usd: number;
  size_quotes: SizeQuote[];
  /** full：本轮全量重算；incremental：复用缓存（池状态未变）。 */
  source: string;
}

export interface GraphStats {
  chain_id: number;
  tokens: number;
  approved_tokens: number;
  pools: number;
  eligible_pools: number;
  edges: number;
  rejected_pools: number;
  built_at: string | null;
  signature: string;
}

export interface ScanRunStats {
  run_id: string;
  started_at: string | null;
  elapsed_ms: number;
  full_rescan: boolean;
  reason: string;
  pools: number;
  eligible_pools: number;
  tokens: number;
  approved_tokens: number;
  edges: number;
  candidates: number;
  requoted: number;
  reused: number;
  visits: number;
  pruned_liquidity: number;
  pruned_risk: number;
  truncated: boolean;
  rejected_by_gate: number;
  profitable: number;
  base_tokens: string[];
  min_hops: number;
  max_hops: number;
  model_level: ModelLevel;
  notes: string[];
}

export interface RouteCacheStats {
  routes: number;
  topology_signature: string;
  empty?: boolean;
}

export interface RouteScanResponse {
  total: number;
  routes: RouteScanRecord[];
  graph: GraphStats;
  scan: ScanRunStats;
}

export interface DexGraphResponse {
  graph: GraphStats;
  scan: ScanRunStats;
  cache: RouteCacheStats;
}

export interface DexEngineParams {
  model_note: string;
  min_hops: number;
  max_hops: number;
  max_candidates: number;
  max_pools: number;
  max_visits: number;
  max_elapsed_ms: number;
  min_input_usd: number;
  max_input_usd: number;
  min_pool_tvl_usd: number;
  max_pool_state_age_ms: number;
  max_pool_share_pct: number;
  grid_points: number;
  refine_iters: number;
  base_symbols: string[];
  notional_ladder: number[];
  flash_fee_bps: number;
  priority_fee_usd: number;
  delay_reserve_bps: number;
  mev_reserve_bps: number;
  profit_discount_pct: number;
  model_mismatch_max_bps: number;
  requote_age_ms: number;
  full_rescan_blocks: number;
  ttl_sec: number;
  gas_price_gwei: number;
  native_price_usd: number;
  stress: {
    adverse_bps: number;
    gas_increase_pct: number;
    flash_fee_increase_pct: number;
  };
}

export interface DexEngineResponse {
  params: DexEngineParams;
  scan: ScanRunStats;
  graph: GraphStats;
  cache: RouteCacheStats;
}

/** 搜索 Token 白/黑名单条目 */
export type SearchTokenEntry = {
  symbol: string;
  address: string;
  decimals: number;
  status: string;
  source: string;
};

/** 可在线修改的搜索参数 */
export type SearchParamsConfig = {
  min_pool_tvl_usd: number;
  max_pool_share_pct: number;
  min_input_usd: number;
  max_input_usd: number;
  max_pool_state_age_ms: number;
  min_hops: number;
  max_hops: number;
  max_pools: number;
  max_evaluate: number;
  max_evaluate_ms: number;
  notional_ladder: number[];
  base_symbols: string[];
  // 规模与算力预算
  max_candidates: number;
  grid_points: number;
  refine_iters: number;
  // 时效
  ttl_sec: number;
  requote_age_ms: number;
  full_rescan_blocks: number;
  // 成本与安全垫
  gas_price_gwei: number;
  priority_fee_usd: number;
  flash_fee_bps: number;
  delay_reserve_bps: number;
  mev_reserve_bps: number;
  profit_discount_pct: number;
  model_mismatch_max_bps: number;
  // 压力模型
  stress_adverse_bps: number;
  stress_gas_increase_pct: number;
};

export type SearchConfigResponse = {
  tokens: SearchTokenEntry[];
  whitelist: number;
  blacklist: number;
  params: SearchParamsConfig;
  effective: Partial<SearchParamsConfig>;
  note: string;
};

export type DiscoveryStatus = {
  enabled: boolean;
  running: boolean;
  last_block: number;
  latest_block: number;
  scanned_logs: number;
  new_pools: number;
  new_tokens: number;
  total_pools: number;
  last_run_at: string;
  message: string;
  source: string;
};

export type SourceEntry = {
  id: string;
  name: string;
  kind: string;
  enabled: boolean;
  priority: number;
  api_key: string;
  url: string;
  rate_limit_per_min: number;
  last_status: string;
  last_used_at: string;
};

export type SourceConfig = {
  strategy: string;
  interval_sec: number;
  sources: SourceEntry[];
};

export type SourcePresetResponse = {
  mode: string;
  warnings: string[];
  config: SourceConfig;
};

/** 标价明细（/api/system/prices） */
export type PriceRow = {
  symbol: string;
  usd: number;
  divergence_bps: number;
};

export type PricesResponse = {
  source: string;
  max_divergence_bps: number;
  notes: string | null;
  tokens: PriceRow[];
};