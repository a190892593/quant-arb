import { Fragment, useState } from "react";
import { data } from "../data";
import { fmt, signClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";
import type { Execution } from "../types";

type Variant = "ok" | "bad" | "warn" | "accent" | "idle" | "plain";
type Tone = "ok" | "bad" | "warn";

interface TimelineEvent {
  t: string;
  title: string;
  tone: Tone;
  note: string;
}

const EXECUTIONS = data.executions;
const EXEC_STATES = data.EXEC_STATES;

const EX_VARIANT: Record<string, [Variant, string]> = {
  HEDGED: ["ok", "HEDGED"],
  RECONCILED: ["ok", "RECONCILED"],
  PARTIAL_HEDGE: ["warn", "PARTIAL_HEDGE"],
  ORDER_STATUS_UNKNOWN: ["warn", "STATUS UNKNOWN"],
  DEX_REVERTED: ["bad", "DEX_REVERTED"],
  HEDGE_FAILED: ["bad", "HEDGE_FAILED"],
  UNWIND_REQUIRED: ["bad", "UNWIND_REQUIRED"],
  UNWIND_FAILED: ["bad", "UNWIND_FAILED"],
  MANUAL_RECOVERY_REQUIRED: ["bad", "MANUAL RECOVERY"],
  REJECTED: ["bad", "REJECTED"],
};

const variantOf = (state: string): [Variant, string] => EX_VARIANT[state] ?? ["idle", state];

const legVariant = (state: string): Variant =>
  state === "FILLED" || state === "CONFIRMED"
    ? "ok"
    : state === "PARTIALLY_FILLED" || state === "UNKNOWN"
      ? "warn"
      : state === "REVERTED"
        ? "bad"
        : "idle";

const toneClassOf = (tone: Tone): string => (tone === "ok" ? "is-ok" : tone === "bad" ? "is-bad" : "is-warn");

function stateIndex(ex: Execution): { i: number; err: boolean } {
  switch (ex.state) {
    case "RECONCILED":
      return { i: EXEC_STATES.length - 1, err: false };
    case "HEDGED":
      return { i: 5, err: false };
    case "PARTIAL_HEDGE":
      return { i: 4, err: true };
    case "ORDER_STATUS_UNKNOWN":
      return { i: 3, err: true };
    case "DEX_REVERTED":
      return { i: 3, err: true };
    case "UNWIND_REQUIRED":
      return { i: 6, err: true };
    case "UNWIND_FAILED":
      return { i: 6, err: true };
    case "MANUAL_RECOVERY_REQUIRED":
      return { i: 6, err: true };
    case "REJECTED":
      return { i: 1, err: true };
    default:
      return { i: 2, err: false };
  }
}

function eventsFor(ex: Execution): TimelineEvent[] {
  const started = ex.startedAt.slice(11, 19);
  const ev: TimelineEvent[] = [
    {
      t: started,
      title: "计划通过风控并生成",
      tone: "ok",
      note: "Notional " + fmt.num(ex.notional, 0) + " USDC · " + ex.order,
    },
  ];
  if (ex.state === "REJECTED") {
    ev.push({ t: started, title: "Pre-Trade Risk Engine 拒绝", tone: "bad", note: ex.statusNote });
    return ev;
  }
  ex.legs.forEach((l) => {
    ev.push({
      t: l.submitted,
      title: l.side + " 腿提交 · " + l.venue,
      tone: l.state === "REVERTED" || l.state === "UNKNOWN" ? "bad" : "ok",
      note: "数量 " + l.qty + (l.price !== "—" ? " · 均价 " + l.price : ""),
    });
    if (l.filled !== "—" && l.filled) {
      ev.push({
        t: l.filled,
        title: l.side + " 腿" + (l.state === "PARTIALLY_FILLED" ? "部分成交" : "成交确认"),
        tone: l.state === "PARTIALLY_FILLED" ? "warn" : "ok",
        note: "费用 " + l.fee + " · 滑点 " + l.slip,
      });
    }
  });
  if (ex.revert) ev.push({ t: started, title: "整笔回滚", tone: "bad", note: ex.revert });
  if (ex.state === "PARTIAL_HEDGE")
    ev.push({
      t: "15:58:45",
      title: "残余敞口触发补偿",
      tone: "warn",
      note: "按新盘口重新报价，补偿损耗 0.34 USDC 计入本次净利",
    });
  if (ex.state === "ORDER_STATUS_UNKNOWN")
    ev.push({
      t: "15:12:12",
      title: "查询确认：订单未成交",
      tone: "warn",
      note: "未重发相同逻辑订单，避免重复成交风险",
    });
  if (ex.state === "UNWIND_REQUIRED") {
    ev.push({
      t: "13:38:24",
      title: "对冲失败：两次 IOC 未成交",
      tone: "bad",
      note: "CEX 腿两次重新报价均未获得成交",
    });
    ev.push({
      t: "13:38:26",
      title: "进入 UNWIND_REQUIRED",
      tone: "bad",
      note: "残余敞口 12.60 USDC 转入补偿队列，暂停该交易对",
    });
  }
  if (ex.state === "MANUAL_RECOVERY_REQUIRED") {
    ev.push({
      t: "12:04:31",
      title: "补偿报价超出保护价",
      tone: "warn",
      note: "两次补偿均突破 10 bps 保护价，放弃追价",
    });
    ev.push({
      t: "12:09:00",
      title: "转入 MANUAL_RECOVERY_REQUIRED",
      tone: "bad",
      note: "敞口 18.20 USDC 冻结，等待人工处置并记录操作者",
    });
  }
  if (ex.state === "HEDGED" || ex.state === "RECONCILED")
    ev.push({
      t: "16:41:24",
      title: "账本核对一致",
      tone: "ok",
      note: "两场所余额变动与模拟成交数量完全匹配，差异 0.00",
    });
  return ev;
}

/** 敞口曲线：原型为 ARB.charts.exposure，这里内联 SVG，采样节奏与刻度完全一致 */
function ExposureStrip({ values, limit }: { values: number[]; limit: number }) {
  const W = 1000;
  const H = 46;
  const max = Math.max(limit * 1.25, ...values);
  const x = (i: number) => (values.length > 1 ? (i / (values.length - 1)) * W : 0);
  const y = (v: number) => H - (v / max) * H;
  const d = values.map((v, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(2)} ${y(v).toFixed(2)}`).join(" ");
  const lim = H - (limit / max) * H;
  return (
    <div className="exposure">
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        style={{ width: "100%", height: H }}
        aria-hidden="true"
      >
        <line
          x1={0}
          y1={lim}
          x2={W}
          y2={lim}
          stroke="var(--neg)"
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
          strokeDasharray="3 3"
        />
        <path d={`${d} L${W} ${H} L0 ${H} Z`} fill="oklch(82% 0.125 82 / .14)" />
        <path d={d} stroke="var(--warn)" strokeWidth={1.4} fill="none" vectorEffect="non-scaling-stroke" />
      </svg>
    </div>
  );
}

export function Executions() {
  const d = data;
  const list = EXECUTIONS;
  const [sel, setSel] = useState("EX-2026-1119-0042");
  const ex = list.find((x) => x.id === sel) ?? list[0];
  const si = stateIndex(ex);
  const events = eventsFor(ex);
  const [exVariant, exLabel] = variantOf(ex.state);

  const done = list.filter((x) => x.state === "HEDGED" || x.state === "RECONCILED").length;
  const abnormal = list.filter(
    (x) =>
      ["DEX_REVERTED", "UNWIND_REQUIRED", "MANUAL_RECOVERY_REQUIRED", "PARTIAL_HEDGE", "ORDER_STATUS_UNKNOWN"].indexOf(
        x.state,
      ) !== -1,
  ).length;
  const rejected = list.filter((x) => x.state === "REJECTED").length;
  const losses = list.filter((x) => x.netPnl < 0).reduce((s, x) => s + x.netPnl, 0);
  const fillRate = (done / list.length) * 100;
  const peak = ex.exposureSeries ? Math.max(...ex.exposureSeries) : 0;

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">04</span> PAPER · EXECUTION
          </div>
          <h1>Paper 执行与对冲</h1>
          <p className="page-sub">
            与未来实盘共用同一状态机：CEX 与 DEX 两腿不存在共同原子提交，任一腿的部分成交、回滚或状态未知都必须显式处置。
          </p>
        </div>
        <div className="page-actions">
          <div className="seg">
            <button className="is-active">今日</button>
            <button>7D</button>
            <button>全部</button>
          </div>
          <button
            className="btn"
            onClick={() => toast("执行报告已生成到本地导出队列", { sub: "arbx-paper-report-20261119.csv" })}
          >
            <Icon name="external" /> 导出执行报告
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">
              今日执行<span className="unit">TRACES</span>
            </div>
            <div className="kpi-value">{list.length}</div>
            <div className="kpi-foot">
              <span className="delta pos">{done}</span>
              <span>完成 · </span>
              <span className="delta warn">{abnormal}</span>
              <span>异常 · </span>
              <span className="delta neg">{rejected}</span>
              <span>拒绝</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              双腿完成率<span className="unit">HEDGE FILL</span>
            </div>
            <div className="kpi-value">
              {fmt.num(fillRate, 1)}
              <small>%</small>
            </div>
            <div className="kpi-foot">
              <span>目标 ≥ 90%</span>
              <span className="kpi-spark" style={{ width: 64 }}>
                <span className={`bar ${fillRate >= 90 ? "bar--ok" : fillRate >= 60 ? "bar--warn" : "bar--bad"}`}>
                  <i style={{ width: `${Math.min(100, fillRate)}%` }} />
                </span>
              </span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              平均未对冲时间<span className="unit">已完成交易 · P95 2.14 s</span>
            </div>
            <div className="kpi-value">
              0.98<small>s</small>
            </div>
            <div className="kpi-foot">
              <span>上限 2.00 s</span>
              <span className="delta">占用 49%</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              补偿与回滚损耗<span className="unit">USDC</span>
            </div>
            <div className="kpi-value neg">{fmt.num(Math.abs(losses))}</div>
            <div className="kpi-foot">
              <span className="delta neg">−0.34</span>
              <span>补偿 · </span>
              <span className="delta neg">−{fmt.num(Math.abs(losses) - 0.34)}</span>
              <span>回滚 / 未对冲</span>
            </div>
          </div>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>
              执行轨迹 <span className="count">{list.length}</span>
            </h2>
            <div className="panel-meta">点击行查看两腿明细</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl tbl-compact">
                <thead>
                  <tr>
                    <th>执行</th>
                    <th>策略</th>
                    <th>路线 / 场所</th>
                    <th className="is-num">名义</th>
                    <th className="is-num">净利</th>
                    <th className="is-num">敞口</th>
                    <th className="is-num">时长</th>
                    <th>执行顺序</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {list.map((x) => {
                    const [v, label] = variantOf(x.state);
                    return (
                      <tr
                        key={x.id}
                        className={`is-link ${x.id === ex.id ? "is-selected" : ""}`}
                        onClick={() => setSel(x.id)}
                      >
                        <td className="cell-id">{x.id}</td>
                        <td>
                          <span className="tag-mini">{d.STRATEGY[x.strategy].short}</span>
                        </td>
                        <td className="cell-strong">
                          {x.pair}
                          <span className="cell-sub">
                            {x.cex ? x.cex + " · " : ""}
                            {d.CHAIN[x.chain].label}
                          </span>
                        </td>
                        <td className="is-num">{fmt.num(x.notional, 0)}</td>
                        <td className={`is-num ${signClass(x.netPnl)}`}>{fmt.signed(x.netPnl)}</td>
                        <td className={`is-num ${x.exposure > 0 ? "warn" : "muted"}`}>{fmt.num(x.exposure)}</td>
                        <td className="is-num muted">{fmt.age(x.duration)}</td>
                        <td className="muted">{x.order}</td>
                        <td>
                          <Chip variant={v} dot>
                            {label}
                          </Chip>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>
              {ex.id} <span className="count">{ex.pair}</span>
            </h2>
            <div className="panel-meta">
              <Chip variant={exVariant} dot>
                {exLabel}
              </Chip>
              <span>
                净利{" "}
                <b
                  style={{ color: ex.netPnl > 0 ? "var(--pos)" : ex.netPnl < 0 ? "var(--neg)" : "var(--ink-2)" }}
                >
                  {fmt.signed(ex.netPnl)}
                </b>{" "}
                USDC
              </span>
            </div>
          </header>
          <div className="panel-body">
            <div className="states">
              {EXEC_STATES.map((s, i) => {
                const isDone = i < si.i;
                const isCur = i === si.i;
                const isErr = isCur && si.err;
                return (
                  <span
                    key={s}
                    className={`state ${isDone ? "is-done" : ""} ${isCur ? (isErr ? "is-error" : "is-current") : ""}`}
                  >
                    {s}
                  </span>
                );
              })}
            </div>
            {si.err ? (
              <div className="note mt-12">
                <b>异常状态：{ex.state}</b>，需人工或补偿流程处置后才可回到主流程。
              </div>
            ) : null}
          </div>
          <div className="panel-body panel-body--flush" style={{ borderTop: "1px solid var(--line-1)" }}>
            <div className="grid g-even" style={{ gap: 1, background: "var(--line-1)" }}>
              <div style={{ background: "var(--bg-2)", padding: 13 }}>
                <div className="eyebrow mb-8">LEGS</div>
                <div className="stack" style={{ gap: 8 }}>
                  {ex.legs.length ? (
                    ex.legs.map((l, i) => (
                      <div className="leg-card" key={`${l.side}-${i}`}>
                        <div className="leg-card-head">
                          <span className={`leg-side ${l.side === "CEX" ? "is-cex" : ""}`}>{l.side}</span>
                          <Chip variant={legVariant(l.state)}>{l.state}</Chip>
                        </div>
                        <div className="muted" style={{ fontSize: 10.5 }}>
                          {l.venue}
                        </div>
                        <dl className="kv">
                          <dt>提交 / 成交</dt>
                          <dd>
                            {l.submitted} → {l.filled}
                          </dd>
                          <dt>数量</dt>
                          <dd>{l.qty}</dd>
                          <dt>均价</dt>
                          <dd>{l.price}</dd>
                          <dt>费用</dt>
                          <dd>{l.fee}</dd>
                          <dt>滑点</dt>
                          <dd>{l.slip}</dd>
                        </dl>
                      </div>
                    ))
                  ) : (
                    <div className="empty">该执行在建单阶段即被风控拒绝，未产生任何交易腿。</div>
                  )}
                </div>

                {ex.exposureSeries ? (
                  <div className="mt-16">
                    <div className="row between mb-8">
                      <span className="eyebrow">未对冲敞口 · 时间序列</span>
                      <span className="mono" style={{ fontSize: 10, color: "var(--warn)" }}>
                        峰值 {fmt.num(peak)} · 上限 25.00
                      </span>
                    </div>
                    <ExposureStrip values={ex.exposureSeries} limit={25} />
                    <div className="hint mt-8">
                      敞口以秒级采样记录；超过 2.00 s 或 25.00 USDC 即触发补偿并暂停该交易对。
                    </div>
                  </div>
                ) : null}
              </div>

              <div style={{ background: "var(--bg-2)", padding: 13 }}>
                <div className="eyebrow mb-8">TIMELINE</div>
                <div className="tl">
                  {events.map((e, i) => (
                    <div className={`tl-item ${toneClassOf(e.tone)}`} key={`${e.t}-${i}`}>
                      <div className="tl-time">{e.t}</div>
                      <div className="tl-title">{e.title}</div>
                      <div className="tl-note">{e.note}</div>
                    </div>
                  ))}
                </div>
                <div className="divider" />
                <div className="eyebrow mb-8">账本影响</div>
                <dl className="kv">
                  {(ex.ledger ?? [{ account: "—", delta: "无分录" }]).map((l, i) => (
                    <Fragment key={`${l.account}-${i}`}>
                      <dt>{l.account}</dt>
                      <dd>{l.delta}</dd>
                    </Fragment>
                  ))}
                </dl>
              </div>
            </div>
          </div>
          <div className="panel-note">{ex.statusNote}</div>
        </section>
      </div>
    </>
  );
}
