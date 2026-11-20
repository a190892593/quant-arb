import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  DexEngineResponse,
  GraphStats,
  InputPoint,
  RouteLifecycle,
  RouteScanRecord,
  RouteType,
  ScanRunStats,
  SimulationOutcome,
  SimulationStatus,
} from "../types";
import { routeDetail, routes, runSimulation, scannerEngine } from "../data/api";
import { fmt, signClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { LineChart } from "../charts/LineChart";
import { Chip, QualityChip, StatusChip } from "../ui/Chip";
import { toast } from "../ui/Toast";
import { SortStack, type SortLevel } from "../ui/SortStack";
import { Drawer } from "../ui/Drawer";
import { actions, data } from "../data";
import { useScanState } from "../data/store";
import { ScanStrip } from "../components/ScanStrip";
import { SearchConfigDrawer } from "../components/SearchConfigDrawer";

const MODEL_NOTE_FALLBACK =
  "M2 未接入 REVM：本机最高模拟等级为 M1 + M3（M2 不可用）；VERIFIED 仅保留给 M2。";

const TYPE_LABEL: Record<RouteType, string> = {
  CROSS_POOL: "跨池",
  TRIANGLE: "三角",
  MULTI_HOP: "多跳",
};

const TYPE_DESC: Record<RouteType, string> = {
  CROSS_POOL: "2 跳 / 2 池",
  TRIANGLE: "3 跳闭环",
  MULTI_HOP: "4 跳及以上",
};

const LIFECYCLE_LABEL: Record<RouteLifecycle, string> = {
  DISCOVERED: "已发现",
  QUOTED: "已报价",
  PROFITABLE_ESTIMATE: "估算盈利",
  SIMULATED: "已模拟",
  VERIFIED: "已验证",
  REJECTED: "已拒绝",
  EXPIRED: "已过期",
  FAILED: "模拟失败",
};

const SIM_LABEL: Record<SimulationStatus, string> = {
  PASSED: "通过",
  FAILED: "失败",
  UNAVAILABLE: "不可用",
  MODEL_MISMATCH: "模型不一致",
};

const SEVERITY_LABEL: Record<string, string> = {
  HARD_STOP: "硬阻断",
  SOFT_BLOCK: "软阻断",
  WARNING: "警告",
  PAPER_ONLY: "仅 Paper",
};

const AMM_LABEL: Record<string, string> = {
  UNISWAP_V2: "Uniswap V2",
  UNISWAP_V3: "Uniswap V3",
  STABLESWAP: "StableSwap",
  WEIGHTED: "Weighted",
  UNKNOWN: "Unknown",
};

type ChipVariant = "ok" | "bad" | "warn" | "accent" | "idle" | "plain";
type TypeFilter = "ALL" | RouteType;
type NetFilter = "ALL" | "GT0" | "GE5" | "GE20";
type HopFilter = "ALL" | "2" | "3" | "4";
/** 拒绝原因截断到一行 */
const clip = (s: string, max = 24) => {
  const t = s.replace(/(\d+\.\d{4})\d+/g, "$1").replace(/\s+/g, " ").trim();
  return t.length > max ? t.slice(0, max) + "…" : t;
};

type SortKey = "net" | "netBps" | "notional" | "gross" | "cost" | "gas" | "hops" | "time";

interface Snapshot {
  routes: RouteScanRecord[];
  graph: GraphStats;
  scan: ScanRunStats;
  total: number;
}

const cx = (...parts: (string | false | null | undefined)[]): string =>
  parts.filter((p): p is string => Boolean(p)).join(" ");

const humanizeProtocol = (id: string): string => {
  const s = id || "";
  if (s.startsWith("uni-v3") || s.startsWith("uniswap-v3")) return "Uniswap V3";
  if (s.startsWith("uni-v2") || s.startsWith("uniswap-v2")) return "Uniswap V2";
  if (s.startsWith("sushi")) return "SushiSwap";
  if (s.startsWith("curve")) return "Curve";
  if (s.startsWith("bal")) return "Balancer";
  if (s.startsWith("aero")) return "Aerodrome";
  return s || "—";
};

const ammLabel = (amm: string): string => AMM_LABEL[amm] ?? (amm ? amm.replace(/_/g, " ") : "—");

const sourceLabel = (source: string): string =>
  source === "incremental" ? "增量复用" : source === "full" ? "全量重算" : source || "—";

const sourceTag = (source: string): string =>
  source === "incremental" ? "INC" : source === "full" ? "FULL" : (source || "—").toUpperCase();

const lifecycleVariant = (life: RouteLifecycle): ChipVariant => {
  if (life === "SIMULATED" || life === "VERIFIED") return "ok";
  if (life === "REJECTED" || life === "FAILED") return "bad";
  if (life === "PROFITABLE_ESTIMATE") return "warn";
  return "idle";
};

const simVariant = (status: SimulationStatus): ChipVariant => {
  if (status === "PASSED") return "ok";
  if (status === "FAILED") return "bad";
  if (status === "MODEL_MISMATCH") return "warn";
  return "idle";
};

const severityVariant = (severity: string): ChipVariant => {
  if (severity === "HARD_STOP") return "bad";
  if (severity === "SOFT_BLOCK" || severity === "WARNING") return "warn";
  return "idle";
};

const clockOf = (iso: string | null | undefined): string => {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : fmt.clock(d);
};

const qty = (n: number): string =>
  fmt.num(n, Math.abs(n) >= 100 ? 2 : Math.abs(n) >= 1 ? 4 : 6);

const proceedsOf = (r: RouteScanRecord): number => r.notional_usd + r.gross_usd;

const valueOf = (r: RouteScanRecord, key: SortKey): number => {
  switch (key) {
    case "net":
      return r.net_usd;
    case "netBps":
      return r.net_bps;
    case "notional":
      return r.notional_usd;
    case "gross":
      return r.gross_usd;
    case "cost":
      return r.total_cost_usd;
    case "gas":
      return r.gas?.usd ?? 0;
    case "hops":
      return r.hops;
    case "time":
      return Date.parse(r.detected_at) || 0;
    default:
      return 0;
  }
};

const sortArrow = (active: boolean, dir: -1 | 1): string => (active ? (dir === 1 ? " ↑" : " ↓") : "");

function SimCard({
  level,
  title,
  sim,
  busy,
  disabled,
  onRun,
}: {
  level: "M1" | "M3";
  title: string;
  sim: SimulationOutcome | null;
  busy: boolean;
  disabled: boolean;
  onRun: () => void;
}) {
  return (
    <div className="route-sim">
      <div className="route-sim-head">
        <div className="row">
          <span className="tag-mini">{level}</span>
          <b>{title}</b>
        </div>
        <div className="row">
          {sim ? (
            <Chip variant={simVariant(sim.status)} dot>
              {SIM_LABEL[sim.status] ?? sim.status}
            </Chip>
          ) : (
            <Chip variant="idle">未运行</Chip>
          )}
          <button
            className="btn btn--sm"
            onClick={onRun}
            disabled={disabled}
            title={"调用 POST /api/simulations 运行 " + level + " 模拟，结果写回后端并落库"}
          >
            <Icon name={busy ? "refresh" : "sim"} />
            {busy ? "运行中" : "运行 " + level}
          </button>
        </div>
      </div>
      {sim ? (
        <>
          <dl className="kv route-sim-kv">
            <dt>报价输出</dt>
            <dd>{qty(sim.quoted_out)}</dd>
            <dt>模拟输出</dt>
            <dd>{qty(sim.simulated_out)}</dd>
            <dt>偏差</dt>
            <dd className={sim.status === "MODEL_MISMATCH" ? "warn" : ""}>
              {fmt.signed(sim.diff_bps, 1) + " bps"}
            </dd>
            <dt>模拟净利</dt>
            <dd className={sim.net_usd == null ? "muted" : signClass(sim.net_usd)}>
              {sim.net_usd == null ? "—" : fmt.signed(sim.net_usd)}
            </dd>
            <dt>Gas</dt>
            <dd>{sim.gas_estimate ? fmt.num(sim.gas_estimate.usd) : "—"}</dd>
            <dt>状态区块</dt>
            <dd>{fmt.num(sim.state_block, 0)}</dd>
            <dt>模拟时间</dt>
            <dd title={sim.created_at}>{clockOf(sim.created_at)}</dd>
          </dl>
          {sim.notes.length > 0 && (
            <ul className="route-sim-notes">
              {sim.notes.map((n, i) => (
                <li key={i}>{n}</li>
              ))}
            </ul>
          )}
          <div className="hint route-sim-id" title={sim.simulation_id}>
            {sim.simulation_id || "无 simulation_id"}
          </div>
        </>
      ) : (
        <div className="hint">
          该路线尚未记录 {level} 结果。运行后由后端重新计算，界面只展示真实返回。
        </div>
      )}
    </div>
  );
}

export function RouteDetail({
  record,
  modelNote,
  onClose,
  onSimulated,
}: {
  record: RouteScanRecord;
  modelNote: string;
  onClose: () => void;
  onSimulated: (level: "M1" | "M3", sim: SimulationOutcome) => void;
}) {
  const [busy, setBusy] = useState<"M1" | "M3" | null>(null);
  const opt = record.optimization;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const points = useMemo(() => {
    const merged = new Map<number, InputPoint>();
    for (const p of record.optimization?.points ?? []) merged.set(p.amount_in, p);
    const best = record.optimization?.best;
    if (best) merged.set(best.amount_in, best);
    return [...merged.values()].sort((a, b) => a.amount_in_usd - b.amount_in_usd);
  }, [record.optimization]);

  const best = opt?.best ?? null;
  const bestIdx = best ? points.findIndex((p) => p.amount_in === best.amount_in) : -1;
  const xLabels =
    points.length > 1
      ? [0, Math.floor((points.length - 1) / 2), points.length - 1].map((i) => ({
          text: fmt.num(points[i].amount_in_usd, 0),
          at: (i / (points.length - 1)) * 100,
        }))
      : [];

  const simulate = async (level: "M1" | "M3") => {
    if (busy) return;
    setBusy(level);
    try {
      const sim = await runSimulation(record.route_id, level);
      onSimulated(level, sim);
      toast(level + " 模拟已返回", {
        sub:
          "状态 " +
          (SIM_LABEL[sim.status] ?? sim.status) +
          " · 偏差 " +
          fmt.num(sim.diff_bps, 1) +
          " bps · " +
          fmt.num(sim.state_block, 0) +
          " 区块",
        variant: sim.status === "PASSED" ? undefined : "warn",
      });
    } catch (e) {
      toast("模拟请求失败", {
        sub: e instanceof Error ? e.message : "后端不可达",
        variant: "bad",
      });
    } finally {
      setBusy(null);
    }
  };

  const fingerprints: [string, string][] = [
    ["有向指纹", record.fingerprints.directed],
    ["无方向指纹", record.fingerprints.undirected],
    ["池序列", record.fingerprints.pool_sequence],
    ["Token 序列", record.fingerprints.token_sequence],
    ["起始资产", record.fingerprints.start_asset],
  ];

  return (
    <Drawer
      ariaLabel="路线详情"
      storageKey="route-detail"
      className="route-drawer"
      onClose={onClose}
      head={
        <>
          <div className="drawer-id">{record.route_id}</div>
          <div className="drawer-sub">
            <span className="tag-mini">{TYPE_LABEL[record.route_type] ?? record.route_type}</span>
            <span>{record.hops + " 跳"}</span>
            <span className="sep" />
            <span>{record.start_token + " 闭环"}</span>
            <span className="sep" />
            <span>{"模型 " + record.model_level}</span>
            <span className="sep" />
            <span>{sourceLabel(record.source)}</span>
          </div>
        </>
      }
      footer={
        <>
          <span className="hint">
            路线记录来自 /api/routes 与 /api/routes/{record.route_id}，不做本地估算。
          </span>
          <button className="btn" onClick={onClose}>
            关闭
          </button>
        </>
      }
    >
        <div className="dw-ribbon">
          {[
            { label: "投入", v: record.notional_usd, bps: null as number | null, note: "名义投入 USDC" },
            { label: "毛利", v: record.gross_usd, bps: null, note: "已扣 AMM 费与冲击" },
            { label: "显式成本", v: -record.total_cost_usd, bps: null, note: "Gas / 优先费 / 储备" },
            { label: "净利", v: record.net_usd, bps: record.net_bps, note: "回款 − 投入 − 成本" },
          ].map((c) => (
            <div className="dw-cell" key={c.label}>
              <span>{c.label}</span>
              <b className={signClass(c.v)}>{fmt.signed(c.v)}</b>
              <small>{c.bps == null ? c.note : fmt.signed(c.bps, 1) + " bps · " + c.note}</small>
            </div>
          ))}
        </div>

        <div className="route-facts">
          <div>
            <span>{"回款（" + record.start_token + "）"}</span>
            <b title={"token 数量 " + record.amount_out}>{qty(record.amount_out)}</b>
          </div>
          <div>
            <span>Gas</span>
            <b>{record.gas ? fmt.num(record.gas.usd) : "—"}</b>
          </div>
          <div>
            <span>路线容量</span>
            <b>{fmt.num(record.capacity_usd, 0)}</b>
          </div>
          <div>
            <span>报价年龄</span>
            <b className={record.quote_age_ms > 1000 ? "neg" : record.quote_age_ms > 250 ? "warn" : ""}>
              {fmt.age(record.quote_age_ms)}
            </b>
          </div>
          <div>
            <span>资金</span>
            <b>{record.flash_funded ? "闪电贷计价" : "自有库存"}</b>
          </div>
          <div>
            <span>区块</span>
            <b>{fmt.num(record.block_number, 0)}</b>
          </div>
          <div>
            <span>检测 / 过期</span>
            <b title={record.detected_at + " → " + record.expires_at}>
              {clockOf(record.detected_at) + " → " + clockOf(record.expires_at)}
            </b>
          </div>
          <div>
            <span>机会 ID</span>
            <b title={record.opportunity_id ?? ""}>{record.opportunity_id ?? "—"}</b>
          </div>
        </div>

        {record.size_quotes.length > 0 && (
          <section className="dw-sec">
            <div className="dw-sec-head">
              资金档位 <span>{record.size_quotes.length} 档</span>
            </div>
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>档位</th>
                    <th>投入</th>
                    <th>回款</th>
                    <th>毛利</th>
                    <th>成本</th>
                    <th>净利</th>
                    <th>bps</th>
                  </tr>
                </thead>
                <tbody>
                  {record.size_quotes.map((q) => (
                    <tr key={q.notional_usd}>
                      <td>{fmt.num(q.notional_usd, 0)}</td>
                      <td>{fmt.num(q.amount_in, 4)}</td>
                      <td>{fmt.num(q.amount_out, 4)}</td>
                      <td className={signClass(q.gross_usd)}>{fmt.signed(q.gross_usd)}</td>
                      <td className="neg">{fmt.signed(-q.total_cost_usd)}</td>
                      <td className={signClass(q.net_usd)}>{fmt.signed(q.net_usd)}</td>
                      <td className={signClass(q.net_bps)}>{fmt.signed(q.net_bps, 1)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}

        <section className="dw-sec">
          <div className="dw-sec-head">
            路线与指纹 <span>{record.chain_id + " · " + record.hops + " 跳"}</span>
          </div>
          <div className="route-path">
            <span className="route-path-label">Token</span>
            <span className="route-path-flow">
              {record.token_path.map((t, i) => (
                <span key={t + "-" + i}>
                  <span className="route-token">{t}</span>
                  {i < record.token_path.length - 1 && <span className="route-arrow">→</span>}
                </span>
              ))}
            </span>
          </div>
          <div className="route-path">
            <span className="route-path-label">Pool</span>
            <span className="route-path-flow">
              {record.pool_path.map((p, i) => (
                <span key={p + "-" + i}>
                  <span className="route-pool" title={p}>
                    {p}
                  </span>
                  {i < record.pool_path.length - 1 && <span className="route-arrow">→</span>}
                </span>
              ))}
            </span>
          </div>
          <div className="hint mt-8">
            {"协议：" + (record.protocols.map(humanizeProtocol).join(" · ") || "—")}
            {"  "}· AMM：{record.amms.map(ammLabel).join(" · ") || "—"}
          </div>
          <div className="route-fp mt-8">
            {fingerprints.map(([k, v]) => (
              <div className="route-fp-row" key={k}>
                <span>{k}</span>
                <b title={v}>{v || "—"}</b>
              </div>
            ))}
          </div>
        </section>

        <section className="dw-sec">
          <div className="dw-sec-head">
            逐腿报价{" "}
            <span>{record.legs.length + " 腿 · 区块 " + fmt.num(record.block_number, 0)}</span>
          </div>
          <div className="tbl-wrap">
            <table className="tbl tbl-compact route-legs">
              <thead>
                <tr>
                  <th>#</th>
                  <th>池</th>
                  <th>协议</th>
                  <th>AMM</th>
                  <th className="is-num">输入</th>
                  <th className="is-num">输出</th>
                  <th className="is-num">费率</th>
                  <th className="is-num">冲击</th>
                  <th className="is-num">池 TVL</th>
                  <th className="is-num">区块</th>
                  <th className="is-num">状态年龄</th>
                </tr>
              </thead>
              <tbody>
                {record.legs.length === 0 && (
                  <tr>
                    <td colSpan={11}>
                      <div className="empty">后端未返回逐腿记录。</div>
                    </td>
                  </tr>
                )}
                {record.legs.map((leg) => (
                  <tr key={String(leg.idx) + "-" + leg.pool_id}>
                    <td className="cell-id">{String(leg.idx + 1).padStart(2, "0")}</td>
                    <td className="route-leg-pool" title={leg.pool_id}>
                      {leg.pool_id}
                    </td>
                    <td>{humanizeProtocol(leg.protocol_id)}</td>
                    <td className="muted">{ammLabel(leg.amm)}</td>
                    <td className="is-num">
                      {qty(leg.amount_in)} <span className="faint">{leg.token_in}</span>
                    </td>
                    <td className="is-num">
                      {qty(leg.amount_out)} <span className="faint">{leg.token_out}</span>
                    </td>
                    <td className="is-num muted">{fmt.num(leg.fee_bps / 100, 2) + "%"}</td>
                    <td className="is-num">{fmt.num(leg.price_impact_bps, 1) + " bps"}</td>
                    <td className="is-num">{fmt.compact(leg.pool_tvl_usd)}</td>
                    <td className="is-num muted">{fmt.num(leg.block_number, 0)}</td>
                    <td
                      className={cx(
                        "is-num",
                        leg.state_age_ms > 3000 ? "neg" : leg.state_age_ms > 250 ? "warn" : "muted",
                      )}
                    >
                      {fmt.age(leg.state_age_ms)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>

        <section className="dw-sec">
          <div className="dw-sec-head">
            投入优化曲线{" "}
            <span>
              {opt
                ? fmt.num(opt.grid_points, 0) + " 网格 · " + fmt.num(opt.refine_iters, 0) + " 次细化"
                : "无记录"}
            </span>
          </div>
          {opt ? (
            <>
              <div className="route-opt-meta">
                <div>
                  <span>最优净利</span>
                  <b className={best ? signClass(best.net_usd) : "muted"}>
                    {best ? fmt.signed(best.net_usd) : "—"}
                  </b>
                  <small>
                    {best
                      ? "投入 " + fmt.num(best.amount_in_usd, 0) + " USDC · " + best.source
                      : "无盈利采样点"}
                  </small>
                </div>
                <div>
                  <span>最小可盈利投入</span>
                  <b>{opt.min_profitable_amount == null ? "—" : qty(opt.min_profitable_amount)}</b>
                  <small>{record.start_token + " · 采样点下限"}</small>
                </div>
                <div>
                  <span>最大可盈利投入</span>
                  <b>{opt.max_profitable_amount == null ? "—" : qty(opt.max_profitable_amount)}</b>
                  <small>{record.start_token + " · 采样点上限"}</small>
                </div>
                <div>
                  <span>最大可执行投入</span>
                  <b>{qty(opt.max_executable_amount)}</b>
                  <small>{record.start_token + " · 容量 " + fmt.num(record.capacity_usd, 0) + " USDC"}</small>
                </div>
              </div>
              {points.length >= 2 ? (
                <div className="route-chart">
                  <LineChart
                    values={points.map((p) => p.net_usd)}
                    height={180}
                    baseline={0}
                    fillToBaseline
                    yFormat={(v) => fmt.num(v, 0)}
                    hoverFormat={(v) => fmt.signed(v, 2) + " USDC"}
                    hoverLabel={(i) => {
                      const p = points[i];
                      return p ? "投入 " + fmt.num(p.amount_in_usd, 0) + " USDC · " + p.source : "";
                    }}
                    xLabels={xLabels}
                  />
                  <div className="hint route-chart-note">
                    横轴为按投入金额升序的采样点，悬停读取实际投入 USD；参考线为 0 净利。
                  </div>
                </div>
              ) : (
                <div className="empty">采样点不足 2 个，无法绘制曲线；下表仍展示后端返回的全部点。</div>
              )}
              <div className="tbl-wrap mt-12">
                <table className="tbl tbl-compact route-opt-tbl">
                  <thead>
                    <tr>
                      <th className="is-num">投入</th>
                      <th className="is-num">投入 USD</th>
                      <th className="is-num">回款</th>
                      <th className="is-num">毛利</th>
                      <th className="is-num">成本</th>
                      <th className="is-num">净利</th>
                      <th className="is-num">bps</th>
                      <th className="is-num">Gas</th>
                      <th className="is-num">Gas 占比</th>
                      <th className="is-num">冲击</th>
                      <th>来源</th>
                    </tr>
                  </thead>
                  <tbody>
                    {points.map((p, i) => (
                      <tr key={String(p.amount_in) + "-" + i} className={bestIdx === i ? "is-best" : ""}>
                        <td className="is-num">{qty(p.amount_in)}</td>
                        <td className="is-num">{fmt.num(p.amount_in_usd, 0)}</td>
                        <td className="is-num">{qty(p.amount_out)}</td>
                        <td className={cx("is-num", signClass(p.gross_usd))}>{fmt.signed(p.gross_usd)}</td>
                        <td className="is-num neg">{fmt.signed(-p.cost_usd)}</td>
                        <td className={cx("is-num", signClass(p.net_usd))}>{fmt.signed(p.net_usd)}</td>
                        <td className={cx("is-num", signClass(p.net_bps))}>{fmt.signed(p.net_bps, 1)}</td>
                        <td className="is-num muted">{fmt.num(p.gas_usd)}</td>
                        <td className="is-num muted">{fmt.num(p.gas_share_pct, 1) + "%"}</td>
                        <td className="is-num muted">{fmt.num(p.price_impact_bps, 1)}</td>
                        <td>
                          <span className="tag-mini">{p.source}</span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {opt.notes.length > 0 && (
                <ul className="route-opt-notes">
                  {opt.notes.map((n, i) => (
                    <li key={i}>{n}</li>
                  ))}
                </ul>
              )}
              {opt.truncated && (
                <div className="route-opt-warn">优化网格被预算或边界截断，结果不代表全局最优。</div>
              )}
            </>
          ) : (
            <div className="empty">后端未返回投入优化记录。</div>
          )}
        </section>

        <section className="dw-sec">
          <div className="dw-sec-head">
            M1 / M3 模拟 <span>M2 未接入 REVM · 最高 M1 + M3</span>
          </div>
          <div className="route-sims">
            <SimCard
              level="M1"
              title="状态一致性"
              sim={record.simulation}
              busy={busy === "M1"}
              disabled={busy !== null}
              onRun={() => void simulate("M1")}
            />
            <SimCard
              level="M3"
              title="压力情景"
              sim={record.stress}
              busy={busy === "M3"}
              disabled={busy !== null}
              onRun={() => void simulate("M3")}
            />
          </div>
          <div className="hint mt-8">{modelNote}</div>
        </section>

        <section className="dw-sec">
          <div className="dw-sec-head">
            拒绝原因 <span>{record.rejections.length + " 项"}</span>
          </div>
          {record.rejections.length === 0 ? (
            <div className="hint">本路线未触发拒绝规则。</div>
          ) : (
            <div className="tbl-wrap">
              <table className="tbl tbl-compact route-reject-tbl">
                <thead>
                  <tr>
                    <th>规则</th>
                    <th>级别</th>
                    <th className="is-num">阈值</th>
                    <th className="is-num">实际</th>
                    <th>说明</th>
                  </tr>
                </thead>
                <tbody>
                  {record.rejections.map((v, i) => (
                    <tr key={v.key + "-" + i}>
                      <td>
                        <div className="route-reject-key">{v.label}</div>
                        <span className="cell-sub">{v.key}</span>
                      </td>
                      <td>
                        <Chip variant={severityVariant(v.severity)}>
                          {SEVERITY_LABEL[v.severity] ?? v.severity}
                        </Chip>
                      </td>
                      <td className="is-num">
                        {fmt.num(v.limit, 4)} <span className="faint">{v.unit}</span>
                      </td>
                      <td className="is-num">
                        {fmt.num(v.actual, 4)} <span className="faint">{v.unit}</span>
                      </td>
                      <td className="route-msg">{v.message}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>

        <section className="dw-sec">
          <div className="dw-sec-head">
            Gas 分项 <span>{record.gas ? fmt.num(record.gas.units, 0) + " units" : "无记录"}</span>
          </div>
          {record.gas ? (
            <>
              <div className="tbl-wrap">
                <table className="tbl tbl-compact">
                  <thead>
                    <tr>
                      <th>分项</th>
                      <th>key</th>
                      <th className="is-num">Gas units</th>
                    </tr>
                  </thead>
                  <tbody>
                    {record.gas.components.map((c, i) => (
                      <tr key={c.key + "-" + i}>
                        <td>{c.label}</td>
                        <td className="cell-id">{c.key}</td>
                        <td className="is-num">{fmt.num(c.units, 0)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <div className="route-gas-foot">
                <span>合计 {fmt.num(record.gas.units, 0)} units</span>
                <span>{fmt.num(record.gas.gas_price_gwei, 4) + " gwei"}</span>
                <span>{"原生资产 $" + fmt.num(record.gas.native_price_usd, 2)}</span>
                <span className="route-gas-usd">{fmt.num(record.gas.usd) + " USDC"}</span>
                <span>区块 {fmt.num(record.gas.block_number, 0)}</span>
                <span>{clockOf(record.gas.estimated_at)}</span>
              </div>
              {record.gas.note && <div className="hint mt-8">{record.gas.note}</div>}
            </>
          ) : (
            <div className="hint">该路线没有 Gas 估算记录。</div>
          )}
        </section>
    </Drawer>
  );
}

export function RouteSearch() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [engine, setEngine] = useState<DexEngineResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<RouteScanRecord | null>(null);

  const [typeFilter, setTypeFilter] = useState<TypeFilter>("ALL");
  const [netFilter, setNetFilter] = useState<NetFilter>("ALL");
  const [hopFilter, setHopFilter] = useState<HopFilter>("ALL");
  const [chainFilter, setChainFilter] = useState<"ALL" | number>("ALL");
  const [sizeFilter, setSizeFilter] = useState<"ALL" | number>("ALL");
  const [levels, setLevels] = useState<SortLevel<SortKey>[]>([{ key: "time", dir: -1 }]);
  const [statusFilter, setStatusFilter] = useState<"ALL" | "ACTIONABLE" | "WATCH" | "REJECTED">("ALL");
  const { scanDex } = useScanState();
  const [cfgOpen, setCfgOpen] = useState(false);
  // 路线 ↔ 机会 合并：按 opportunity_id 关联，机会侧补上风险分、数据质量与处置状态
  const oppByRouteId = new Map(data.opps.map((o) => [o.rawId ?? o.id, o]));
  // 事件提示用真实数字，避免演示常量误导
  const liveEvents = [
    { tone: "ok" as const, text: `${data.scanner.quotes.toLocaleString("en-US")} quotes/s · ${data.pools.length} 池在册` },
    { tone: "info" as const, text: `机会 ${data.opps.length} 条 · 可执行 ${data.opps.filter((o) => o.status === "ACTIONABLE").length}` },
    { tone: "ok" as const, text: `链头 ${(data.chains[0]?.block ?? 0).toLocaleString("en-US")} · 扫描耗时 ${data.scanner.lastScanMs ?? 0} ms` },
    { tone: "warn" as const, text: `拒绝 ${data.scanner.rejected} · 观察 ${data.scanner.watch}` },
    { tone: "ok" as const, text: `已执行 ${data.scanner.executed} · 状态 ${data.scanner.state}` },
  ];

  const engineRef = useRef<DexEngineResponse | null>(null);
  const inFlight = useRef(false);
  const selectedIdRef = useRef<string | null>(null);

  const pull = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const res = await routes({ limit: 100 });
      setSnap({ routes: res.routes, graph: res.graph, scan: res.scan, total: res.total });
      setError(null);
      setUpdatedAt(Date.now());
      setLoading(false);
      if (!engineRef.current) {
        try {
          const eng = await scannerEngine();
          engineRef.current = eng;
          setEngine(eng);
        } catch {
          /* 参数说明缺失时使用静态回退文案，不影响路线数据 */
        }
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "后端不可达");
      setLoading(false);
    } finally {
      inFlight.current = false;
    }
  }, []);

  useEffect(() => {
    void pull();
    const timer = window.setInterval(() => {
      if (!document.hidden) void pull();
    }, 2500);
    const onVisibility = () => {
      if (!document.hidden) void pull();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [pull]);

  const graph = snap?.graph ?? engine?.graph ?? null;
  const scan = snap?.scan ?? engine?.scan ?? null;
  const all = snap?.routes ?? [];
  const updatedLabel = updatedAt ? clockOf(new Date(updatedAt).toISOString()) : null;

  const filtered = useMemo(() => {
    const list = all
      .filter((r) => chainFilter === "ALL" || r.chain_id === chainFilter)
      .filter((r) =>
        sizeFilter === "ALL"
          ? true
          : r.size_quotes.some((q) => Math.round(q.notional_usd) === sizeFilter),
      )
      .filter((r) => typeFilter === "ALL" || r.route_type === typeFilter)
      .filter((r) => {
        if (netFilter === "ALL") return true;
        if (netFilter === "GT0") return r.net_usd > 0;
        if (netFilter === "GE5") return r.net_usd >= 5;
        return r.net_usd >= 20;
      })
      .filter((r) => {
        if (hopFilter === "ALL") return true;
        if (hopFilter === "2") return r.hops === 2;
        if (hopFilter === "3") return r.hops === 3;
        return r.hops >= 4;
      })
      .filter((r) => {
        if (statusFilter === "ALL") return true;
        const opp = r.opportunity_id ? oppByRouteId.get(r.opportunity_id) : undefined;
        return opp ? opp.status === statusFilter : r.lifecycle === "REJECTED" && statusFilter === "REJECTED";
      });
    return [...list].sort((a, b) => {
      for (const lv of levels) {
        const d = valueOf(a, lv.key) - valueOf(b, lv.key);
        if (d !== 0) return d * lv.dir;
      }
      return b.net_usd - a.net_usd;
    });
  }, [all, chainFilter, sizeFilter, typeFilter, netFilter, hopFilter, statusFilter, levels, oppByRouteId]);

  const chainIds = [...new Set(all.map((r) => r.chain_id))].sort((a, b) => a - b);
  const filtersActive =
    chainFilter !== "ALL" ||
    sizeFilter !== "ALL" ||
    typeFilter !== "ALL" ||
    netFilter !== "ALL" ||
    hopFilter !== "ALL" ||
    statusFilter !== "ALL";

  const emptyText =
    error && !snap
      ? "后端不可达：" +
        error +
        "。尚未获得任何真实路线数据；页面每 2.5 秒重试，恢复前不展示估算值。"
      : all.length > 0 && filtered.length === 0
        ? "当前筛选条件下没有路线（共 " +
          all.length +
          " 条）。清除类型 / 最小净利 / 跳数筛选可恢复。"
        : scan?.run_id
          ? "后端当前没有路线：本轮候选 " +
            scan.candidates +
            "，访问 " +
            scan.visits +
            "，剪枝 " +
            (scan.pruned_liquidity + scan.pruned_risk) +
            "，门禁拒绝 " +
            scan.rejected_by_gate +
            "。"
          : "后端尚未产生扫描轮次（run_id 为空）。路由引擎仅在扫描运行且池状态合格时写入结果；页面不会用演示数据填充。";

  const onSort = (key: SortKey) => {
    setLevels((prev) => {
      const hit = prev.find((l) => l.key === key);
      if (hit && prev[0]?.key === key) {
        return prev.map((l, i) => (i === 0 ? { ...l, dir: (l.dir === 1 ? -1 : 1) as -1 | 1 } : l));
      }
      const rest = prev.filter((l) => l.key !== key);
      return [{ key, dir: hit ? hit.dir : -1 }, ...rest];
    });
  };

  const openRoute = (record: RouteScanRecord) => {
    selectedIdRef.current = record.route_id;
    setSelected(record);
    routeDetail(record.route_id)
      .then((fresh) => {
        if (fresh.route_id === selectedIdRef.current) setSelected(fresh);
      })
      .catch(() => {
        /* 列表记录已是真实数据，详情拉取失败时保留列表快照 */
      });
  };

  const onSimulated = (level: "M1" | "M3", sim: SimulationOutcome) => {
    setSelected((prev) =>
      prev
        ? {
            ...prev,
            simulation: level === "M1" ? sim : prev.simulation,
            stress: level === "M3" ? sim : prev.stress,
          }
        : prev,
    );
  };

  const clearFilters = () => {
    setTypeFilter("ALL");
    setNetFilter("ALL");
    setHopFilter("ALL");
    setStatusFilter("ALL");
    setChainFilter("ALL");
    setSizeFilter("ALL");
  };

  const modelNote = engine?.params.model_note || MODEL_NOTE_FALLBACK;

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">02</span> SEARCH · DEX–DEX
          </div>
          <h1>DEX–DEX 搜索引擎</h1>
          <p className="page-sub">
            多池多跳扫描的记录视图：图构建 → 候选枚举 → 逐腿报价 → 投入优化 → M1 状态一致性 / M3
            压力模拟。净利口径为「回款 − 投入 − 全部显式成本」，AMM 费用与冲击已含在成交价内。
          </p>
        </div>
        <div className="page-actions">
          <div className={cx("route-live", error && "is-off")}>
            <span className={cx("scan-pulse", error && "scan-pulse--off")} />
            <b>{error ? "后端不可达" : "LIVE"}</b>
            <span>{error ? "保留上次快照" : "2.5s 轮询"}</span>
          </div>
          <button
            className="btn btn--sm"
            onClick={() => void pull()}
            disabled={loading}
            title="立即请求 /api/routes"
          >
            <Icon name="refresh" />
            刷新
          </button>
          <button className="btn btn--sm" onClick={() => setCfgOpen(true)} title="打开搜索配置抽屉">
            <Icon name="system" />
            搜索配置
          </button>
          <button
            className={`btn btn--sm ${scanDex ? "btn--danger" : "btn--primary"}`}
            onClick={() => void actions.setScan(!scanDex)}
            title={scanDex ? "停止链上扫描" : "启动链上扫描"}
          >
            <Icon name={scanDex ? "pause" : "play"} />
            {scanDex ? "停止搜索" : "启动搜索"}
          </button>
        </div>
      </div>

      <ScanStrip
        running={scanDex}
        subjects={
          data.pools.length
            ? data.pools.slice(0, 4).map((p) => `${p.chain} · ${p.protocol} · ${p.pair}`)
            : ["Arbitrum · 等待池状态"]
        }
        events={liveEvents}
        quotes={data.scanner.quotes || 0}
        pools={data.scanner.pools || data.pools.length}
        block={data.chains[0]?.block ?? 0}
        latency={data.scanner.lastScanMs ?? 0}
        latencyLabel="本轮扫描"
      />

      {error && (
        <div className="route-offline" role="status">
          <Icon name="alert" />
          <div>
            <b>后端不可达</b>
            <span>
              {error}。
              {snap
                ? "界面保留 " + (updatedLabel ?? "上一次") + " 的真实快照，"
                : "尚未获得真实数据，"}
              每 2.5 秒自动重试；不会生成或补造任何路线。
            </span>
          </div>
        </div>
      )}

      <div className="route-engine-note">
        <Icon name="alert" />
        <div>
          <b>模拟能力</b>
          <span>{modelNote}</span>
        </div>
        <span className="tag-mini">{scan?.model_level ?? "M1"}</span>
      </div>

      <SearchConfigDrawer open={cfgOpen} onClose={() => setCfgOpen(false)} />

      <div className="kpi-grid route-kpis">
        <div className="kpi">
          <div className="kpi-label">
            池 / 边 / Token<span className="unit">GRAPH</span>
          </div>
          <div className="kpi-value">
            {graph ? graph.pools : "—"}
            <small>/ {graph ? graph.edges : "—"}</small>
            <small>/ {graph ? graph.tokens : "—"}</small>
          </div>
          <div className="kpi-foot">
            <span>
              合格池 {graph ? graph.eligible_pools : "—"} · 批准 Token{" "}
              {graph ? graph.approved_tokens : "—"} · 拒绝池 {graph ? graph.rejected_pools : "—"}
            </span>
          </div>
        </div>

        <div className="kpi">
          <div className="kpi-label">
            候选路线<span className="unit">CANDIDATES</span>
          </div>
          <div className="kpi-value">{scan ? fmt.num(scan.candidates, 0) : "—"}</div>
          <div className="kpi-foot">
            <span>
              可盈利 {scan ? scan.profitable : "—"} · 落库 {all.length} / 返回 {snap?.total ?? "—"}
            </span>
          </div>
        </div>

        <div className="kpi">
          <div className="kpi-label">
            访问量<span className="unit">VISITS</span>
          </div>
          <div className="kpi-value">{scan ? fmt.num(scan.visits, 0) : "—"}</div>
          <div className="kpi-foot">
            <span>
              剪枝：流动性 {scan ? scan.pruned_liquidity : "—"} · 风险{" "}
              {scan ? scan.pruned_risk : "—"}
            </span>
          </div>
        </div>

        <div className="kpi">
          <div className="kpi-label">
            本轮重算 / 复用<span className="unit">REQUOTE / REUSE</span>
          </div>
          <div className="kpi-value">
            {scan ? scan.requoted : "—"}
            <small>/ {scan ? scan.reused : "—"}</small>
          </div>
          <div className="kpi-foot">
            <span>
              {scan ? (scan.full_rescan ? "全量重扫" : "增量重扫") : "—"} · {scan?.reason || "—"}
            </span>
          </div>
        </div>

        <div className="kpi">
          <div className="kpi-label">
            耗时<span className="unit">ELAPSED</span>
          </div>
          <div className="kpi-value">
            {scan ? fmt.num(scan.elapsed_ms, 1) : "—"}
            <small>ms</small>
          </div>
          <div className="kpi-foot">
            <span>
              预算 {engine ? fmt.num(engine.params.max_elapsed_ms, 0) : "—"} ms · 起点{" "}
              {scan?.base_tokens.length ? scan.base_tokens.join(" / ") : "—"}
            </span>
          </div>
        </div>

        <div className="kpi">
          <div className="kpi-label">
            是否截断<span className="unit">TRUNCATED</span>
          </div>
          <div className={cx("kpi-value", scan?.truncated && "warn")}>
            {scan ? (scan.truncated ? "是" : "否") : "—"}
          </div>
          <div className="kpi-foot">
            <span>
              门禁拒绝 {scan ? scan.rejected_by_gate : "—"} · 候选上限{" "}
              {engine ? fmt.num(engine.params.max_candidates, 0) : "—"}
            </span>
          </div>
        </div>
      </div>

      <section className="panel mt-12">
        <header className="panel-head">
          <h2>
            路线扫描结果{" "}
            <span className="count">
              {filtered.length} / {all.length}
            </span>
          </h2>
          <div className="panel-meta">
            {updatedLabel ? "SYNC " + updatedLabel : loading ? "LOADING" : "NO DATA"}
          </div>
        </header>

        <div className="toolbar">
          <div className="filters">
            <select
              value={chainFilter}
              onChange={(e) =>
                setChainFilter(e.target.value === "ALL" ? "ALL" : Number(e.target.value))
              }
              aria-label="链筛选"
              style={{ height: 28, padding: "0 6px" }}
            >
              <option value="ALL">全部链</option>
              {chainIds.map((id) => (
                <option key={id} value={id}>
                  {id}
                </option>
              ))}
            </select>
            <select
              value={sizeFilter}
              onChange={(e) =>
                setSizeFilter(e.target.value === "ALL" ? "ALL" : Number(e.target.value))
              }
              aria-label="资金档位筛选"
              style={{ height: 28, padding: "0 6px" }}
            >
              <option value="ALL">全部档位</option>
              {[250, 500, 1000, 2500, 5000, 10000].map((v) => (
                <option key={v} value={v}>
                  {v.toLocaleString("en-US")} USD
                </option>
              ))}
            </select>
            <div className="seg" role="group" aria-label="路线类型筛选">
              {(["ALL", "CROSS_POOL", "TRIANGLE", "MULTI_HOP"] as const).map((v) => (
                <button
                  key={v}
                  className={typeFilter === v ? "is-active" : ""}
                  onClick={() => setTypeFilter(v)}
                >
                  {v === "ALL" ? "全部类型" : TYPE_LABEL[v]}
                </button>
              ))}
            </div>
            <div className="seg" role="group" aria-label="最小净利筛选">
              {(["ALL", "GT0", "GE5", "GE20"] as const).map((v) => (
                <button
                  key={v}
                  className={netFilter === v ? "is-active" : ""}
                  onClick={() => setNetFilter(v)}
                >
                  {v === "ALL" ? "净利不限" : v === "GT0" ? "> 0" : "≥ " + (v === "GE5" ? 5 : 20) + " USDC"}
                </button>
              ))}
            </div>
            <div className="seg" role="group" aria-label="跳数筛选">
              {(["ALL", "2", "3", "4"] as const).map((v) => (
                <button
                  key={v}
                  className={hopFilter === v ? "is-active" : ""}
                  onClick={() => setHopFilter(v)}
                >
                  {v === "ALL" ? "跳数不限" : v === "4" ? "4+" : v + " 跳"}
                </button>
              ))}
            </div>
            <div className="seg" role="group" aria-label="状态筛选">
              {([
                ["ALL", "全部状态"],
                ["ACTIONABLE", "可执行"],
                ["WATCH", "观察"],
                ["REJECTED", "拒绝"],
              ] as const).map(([v, label]) => (
                <button
                  key={v}
                  className={statusFilter === v ? "is-active" : ""}
                  onClick={() => setStatusFilter(v)}
                >
                  {label}
                </button>
              ))}
            </div>
            {filtersActive && (
              <button className="btn btn--sm btn--ghost" onClick={clearFilters}>
                <Icon name="x" />
                清除筛选
              </button>
            )}
          </div>
          <SortStack
            levels={levels}
            onChange={setLevels}
            options={[
              { key: "time", label: "出现时间" },
              { key: "net", label: "净利" },
              { key: "netBps", label: "净利 bps" },
              { key: "notional", label: "投入" },
              { key: "gross", label: "毛利" },
              { key: "cost", label: "显式成本" },
              { key: "gas", label: "Gas" },
              { key: "hops", label: "跳数" },
            ]}
          />          <span className="hint">点击行查看逐腿、指纹、优化曲线与模拟</span>
        </div>

        <div className="panel-body panel-body--flush">
          <div className="tbl-wrap">
            <table className="tbl route-tbl">
              <thead>
                <tr>
                  <th>类型</th>
                  <th className="is-num">链</th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("time")}>
                    出现{sortArrow(levels[0]?.key === "time", levels[0]?.dir ?? -1)}
                  </th>
                  <th>Token 闭环路径</th>
                  <th>Pool 路径</th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("hops")}>
                    跳数{sortArrow(levels[0]?.key === "hops", levels[0]?.dir ?? -1)}
                  </th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("notional")}>
                    投入 USD{sortArrow(levels[0]?.key === "notional", levels[0]?.dir ?? -1)}
                  </th>
                  <th className="is-num">回款 USD</th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("gross")}>
                    毛利{sortArrow(levels[0]?.key === "gross", levels[0]?.dir ?? -1)}
                  </th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("cost")}>
                    显式成本{sortArrow(levels[0]?.key === "cost", levels[0]?.dir ?? -1)}
                  </th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("net")}>
                    净利{sortArrow(levels[0]?.key === "net", levels[0]?.dir ?? -1)}
                  </th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("netBps")}>
                    bps{sortArrow(levels[0]?.key === "netBps", levels[0]?.dir ?? -1)}
                  </th>
                  <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("gas")}>
                    Gas{sortArrow(levels[0]?.key === "gas", levels[0]?.dir ?? -1)}
                  </th>
                  <th className="is-num">压力净利</th>
                  <th className="is-num">风险</th>
                  <th>数据</th>
                  <th>状态</th>
                  <th>M1 / M3</th>
                  <th>来源</th>
                </tr>
              </thead>
              <tbody>
                {filtered.length === 0 && (
                  <tr>
                    <td colSpan={14}>
                      <div className="empty">{emptyText}</div>
                    </td>
                  </tr>
                )}
                {filtered.map((r) => {
                  const m1 = r.simulation;
                  const m3 = r.stress;
                  const opp = r.opportunity_id ? oppByRouteId.get(r.opportunity_id) : undefined;
                  return (
                    <tr
                      key={r.route_id}
                      className={cx("is-link", selected?.route_id === r.route_id && "is-selected")}
                      onClick={() => openRoute(r)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          openRoute(r);
                        }
                      }}
                      tabIndex={0}
                      aria-selected={selected?.route_id === r.route_id}
                    >
                      <td>
                        <Chip variant="plain">{TYPE_LABEL[r.route_type] ?? r.route_type}</Chip>
                        <span className="cell-sub">{TYPE_DESC[r.route_type] ?? ""}</span>
                      </td>
                      <td className="is-num muted">{r.chain_id}</td>
                      <td className="is-num muted" title={r.detected_at}>{fmt.hhmmss(r.detected_at)}</td>
                      <td className="route-path-cell">
                        <div className="route-path-inline">
                          {r.token_path.map((t, i) => (
                            <span key={t + "-" + i}>
                              <span className="route-token">{t}</span>
                              {i < r.token_path.length - 1 && <span className="route-arrow">→</span>}
                            </span>
                          ))}
                        </div>
                        <span className="cell-sub">{r.protocols.map(humanizeProtocol).join(" · ")}</span>
                      </td>
                      <td className="route-pool-cell" title={r.pool_path.join(" → ")}>
                        <span className="route-pool-text">{r.pool_path.join(" → ") || "—"}</span>
                      </td>
                      <td className="is-num">{r.hops}</td>
                      <td className="is-num">{fmt.num(r.notional_usd, 0)}</td>
                      <td className="is-num">{fmt.num(proceedsOf(r), 0)}</td>
                      <td className={cx("is-num", signClass(r.gross_usd))}>{fmt.signed(r.gross_usd)}</td>
                      <td className="is-num neg">{fmt.signed(-r.total_cost_usd)}</td>
                      <td className={cx("is-num", signClass(r.net_usd))}>{fmt.signed(r.net_usd)}</td>
                      <td className={cx("is-num", signClass(r.net_bps))}>{fmt.signed(r.net_bps, 1)}</td>
                      <td className="is-num muted">{r.gas ? fmt.num(r.gas.usd) : "—"}</td>
                      <td className={cx("is-num", r.stress && signClass(r.stress.net_usd ?? 0))}>
                        {r.stress ? fmt.signed(r.stress.net_usd) : "—"}
                      </td>
                      <td className={cx("is-num", opp ? (opp.riskScore >= 70 ? "pos" : opp.riskScore >= 50 ? "" : "neg") : "muted")}>
                        {opp ? opp.riskScore : "—"}
                      </td>
                      <td>{opp ? <QualityChip quality={opp.quality} /> : <span className="muted">—</span>}</td>
                      <td>
                        {opp ? (
                          <>
                            <StatusChip status={opp.status} />
                            {opp.reasons && opp.reasons.length > 0 && (
                              <span className="cell-sub" title={opp.reasons.join(" · ")}>
                                {clip(opp.reasons[0], 22)}
                              </span>
                            )}
                          </>
                        ) : (
                          <>
                            <Chip variant={lifecycleVariant(r.lifecycle)} dot>
                              {LIFECYCLE_LABEL[r.lifecycle] ?? r.lifecycle}
                            </Chip>
                            {r.rejections.length > 0 && (
                              <span className="cell-sub">{r.rejections.length + " 项拒绝"}</span>
                            )}
                          </>
                        )}
                      </td>
                      <td>
                        <div className="route-simstats">
                          <span
                            className={cx("route-simstat", m1 ? "is-" + m1.status.toLowerCase() : "is-none")}
                          >
                            <em>M1</em>
                            {m1 ? SIM_LABEL[m1.status] ?? m1.status : "未运行"}
                          </span>
                          <span
                            className={cx("route-simstat", m3 ? "is-" + m3.status.toLowerCase() : "is-none")}
                          >
                            <em>M3</em>
                            {m3 ? SIM_LABEL[m3.status] ?? m3.status : "未运行"}
                          </span>
                        </div>
                      </td>
                      <td>
                        <span className="tag-mini" title={sourceLabel(r.source)}>
                          {sourceTag(r.source)}
                        </span>
                        <span className="cell-sub">{r.model_level}</span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
        <div className="panel-note">
          路线来自后端路由引擎的真实扫描结果；回款 USD = 投入 + 毛利（按起始资产计价），显式成本不含已内含在成交价里的
          AMM 费。点击行的详情栏按需请求 /api/routes/&lt;id&gt;，失败时保留列表快照。
        </div>
      </section>

      {selected && (
        <RouteDetail
          record={selected}
          modelNote={modelNote}
          onClose={() => {
            selectedIdRef.current = null;
            setSelected(null);
          }}
          onSimulated={onSimulated}
        />
      )}


    </>
  );
}
