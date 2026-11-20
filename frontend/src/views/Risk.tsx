import { Fragment, useState } from "react";
import { data, actions } from "../data";
import type { RiskParam, ScopeItem } from "../types";
import { fmt, toneClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

const d = data;

/* ---------- helpers ported from web/js/views/risk.js ---------- */

/** 占用率 → 语义色：≥85 red，≥60 amber（复用全局 toneClass） */
const tone = (pct: number): string => toneClass(pct);

/** 阈值 / 当前值：毫秒与百分比取整，其余保留两位 */
const num = (v: number, unit: string): string =>
  unit === "ms" || unit === "%" ? fmt.num(v, 0) : fmt.num(v);

/** 占用单元格：参照型、零容忍型与「下限型」阈值各有独立语义 */
function usageCell(p: RiskParam) {
  if (p.dir === "ref" || p.cur === null) {
    return (
      <span className="faint mono" style={{ fontSize: 10 }}>
        {p.cur === null ? "未启用" : "—"}
      </span>
    );
  }
  if (p.dir === "zero") {
    const ok = p.cur === 0;
    return (
      <div className="usage-cell">
        <span className={ok ? "bar bar--ok" : "bar bar--bad"}>
          <i style={{ width: `${ok ? 4 : 100}%` }} />
        </span>
        <span className={"usage-num" + (ok ? " pos" : " neg")}>{ok ? "OK" : "FAIL"}</span>
      </div>
    );
  }
  const pct = p.pct ?? 0;
  const width = Math.max(0, Math.min(100, pct));
  const t = p.dir === "min" ? (pct >= 100 ? "ok" : pct >= 60 ? "warn" : "bad") : tone(pct);
  return (
    <div className="usage-cell">
      <span className={t ? `bar bar--${t}` : "bar"}>
        <i style={{ width: `${width}%` }} />
      </span>
      <span className={"usage-num" + (t === "bad" ? " neg" : t === "ok" ? " pos" : "")}>
        {fmt.pct(pct, 0)}
      </span>
    </div>
  );
}

const levelVariant = (level: string): "bad" | "warn" | "idle" =>
  level === "HARD_STOP" ? "bad" : level === "SOFT_BLOCK" ? "warn" : "idle";

const sevVariant = (sev: string): "bad" | "warn" | "idle" | "plain" =>
  sev === "HARD STOP" ? "bad" : sev === "SOFT BLOCK" ? "warn" : sev === "WARNING" ? "idle" : "plain";

function initialScopes(): Record<string, boolean> {
  const out: Record<string, boolean> = {};
  d.scopes.forEach((g) => g.items.forEach((it) => (out[it.id] = it.on)));
  return out;
}

export function Risk() {
  // 暂停状态来自后端（硬风控由后端执行），前端只发起请求
  const paused = data.riskStatus?.globalPaused ?? false;
  const [scopeOn, setScopeOn] = useState<Record<string, boolean>>(initialScopes);
  const totalParams = d.riskGroups.reduce((sum, g) => sum + g.items.length, 0);

  /** 全局执行开关：以服务端状态为准 */
  const togglePause = async () => {
    const next = !paused;
    try {
      await actions.setPaused(next);
      toast(next ? "全局暂停已生效" : "全局执行已恢复", { variant: next ? "bad" : undefined });
    } catch (e) {
      toast("风控状态切换失败", { sub: e instanceof Error ? e.message : String(e), variant: "warn" });
    }
  };

  const onScope = async (item: ScopeItem) => {
    if (item.note) {
      toast("该范围未启用，无法切换", { sub: "需先完成适配器与链路准入", variant: "warn" });
      return;
    }
    if (item.id === "global") {
      const next = !paused;
      try {
        await actions.setPaused(next);
        toast(next ? "全局暂停已生效" : "全局执行已恢复", {
          sub: next ? "所有策略停止生成新计划" : "恢复前已重新校验账本与数据完整性",
          variant: next ? "bad" : undefined,
        });
      } catch (e) {
        toast("风控状态切换失败", { sub: e instanceof Error ? e.message : String(e), variant: "warn" });
      }
      return;
    }
    const next = !scopeOn[item.id];
    setScopeOn((s) => ({ ...s, [item.id]: next }));
    toast((next ? "已启用 " : "已暂停 ") + item.label);
  };

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">11</span> GOVERN · RISK
          </div>
          <h1>风控台</h1>
          <p className="page-sub">
            硬风控由后端强制执行，前端开关无法绕过。所有拒绝与熔断都记录原因、参数版本与数据时间。
          </p>
        </div>
        <div className="page-actions">
          <span className="chip chip--idle">{data.riskStatus?.paramVersion ?? "PARAM v1.0"}</span>
          <button className="btn btn--danger" onClick={togglePause}>
            <Icon name={paused ? "play" : "pause"} />
            {paused ? "恢复执行" : "全局暂停"}
          </button>
        </div>
      </div>

      <div className="stack">
        <div className={`killswitch${paused ? " is-paused" : ""}`}>
          <div className="killswitch-main">
            <span
              className={`dot ${paused ? "dot--bad" : "dot--ok"}`}
              style={{ width: 9, height: 9 }}
            />
            <div>
              <div className="killswitch-state">{paused ? "PAUSED" : "ARMED"}</div>
              <div className="killswitch-sub">
                {paused
                  ? "全局暂停已生效：所有策略停止生成可执行计划，已有持仓与 Paper 订单仍可查询。"
                  : "全链路监控中：DEX 原子套利与数据源健康检查均正常。"}
              </div>
            </div>
          </div>
          <div className="row" style={{ gap: 16 }}>
            <div style={{ textAlign: "right" }}>
              <div className="eyebrow">今日拒绝</div>
              <div className="mono" style={{ fontSize: 15 }}>4</div>
            </div>
            <div style={{ textAlign: "right" }}>
              <div className="eyebrow">熔断</div>
              <div className="mono" style={{ fontSize: 15 }}>1</div>
            </div>
            <div style={{ textAlign: "right" }}>
              <div className="eyebrow">未对冲敞口</div>
              <div className="mono" style={{ fontSize: 15 }}>3.40</div>
            </div>
          </div>
        </div>

        <div className="grid g-main">
          <section className="panel">
            <header className="panel-head">
              <h2>熔断范围</h2>
              <div className="panel-meta">开关仅影响新机会，不改变已持久化记录</div>
            </header>
            <div className="panel-body">
              <div className="stack" style={{ gap: 14 }}>
                {d.scopes.map((g) => (
                  <div key={g.group}>
                    <div className="eyebrow mb-8">{g.group}</div>
                    <div className="scope-grid">
                      {g.items.map((it) => {
                        const active = scopeOn[it.id] ?? it.on;
                        const pressed = it.id === "global" ? !paused : active;
                        return (
                          <div
                            key={it.id}
                            className={`scope${active ? "" : " is-off"}${it.id === "global" ? " is-global" : ""}`}
                          >
                            <span className="scope-name">
                              {it.label}
                              {it.note && <span className="scope-note">{it.note}</span>}
                            </span>
                            <button
                              className="toggle"
                              role="switch"
                              aria-pressed={pressed}
                              aria-label={`${it.label} 执行开关`}
                              onClick={() => onScope(it)}
                            />
                          </div>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>
              <div className="divider" style={{ margin: "14px 0 11px" }} />
              <div className="eyebrow mb-8">恢复前置条件</div>
              <div className="resume-grid">
                {["账本对账差异 0.00", "主备 RPC 区块一致", "盘口 Snapshot 完整", "原因与操作者已记录"].map((t) => (
                  <span className="resume-item" key={t}>
                    <Icon name="check" size={11} />
                    {t}
                  </span>
                ))}
              </div>
            </div>
            <div className="panel-note">
              覆盖规则不得突破全局硬限制：某策略可以把日亏损从 1% 收紧到 0.5%，但不能放宽到 2%。
            </div>
          </section>

          <div className="stack">
            <section className="panel">
              <header className="panel-head">
                <h2>关键阈值</h2>
                <div className="panel-meta">占用 %</div>
              </header>
              <div className="panel-body">
                <div className="stack" style={{ gap: 12 }}>
                  {d.riskParams.slice(0, 6).map((p) => {
                    const t = tone(p.pct ?? 0);
                    return (
                      <div className="meter-row" key={p.key}>
                        <span className="meter-label">{p.key}</span>
                        <span className="meter-val">
                          {fmt.num(p.cur)} / {fmt.num(p.limit)}
                        </span>
                        <div className={t ? `bar bar--${t}` : "bar"}>
                          <i style={{ width: `${Math.min(100, p.pct ?? 0)}%` }} />
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>风险事件</h2>
                <div className="panel-meta">今日 {d.riskEvents.length}</div>
              </header>
              <div className="panel-body panel-body--flush">
                <div className="ev-list">
                  {d.riskEvents.slice(0, 4).map((e) => (
                    <div className="ev" key={e.id}>
                      <div className="ev-top">
                        <Chip variant={levelVariant(e.level)}>{e.level}</Chip>
                        <span className="ev-scope">{e.scope}</span>
                        <span className="ev-time">{e.ts}</span>
                      </div>
                      <div className="ev-msg">{e.msg}</div>
                    </div>
                  ))}
                </div>
              </div>
            </section>
          </div>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>
              参数基线 <span className="count">{totalParams}</span>
            </h2>
            <div className="panel-meta">六级分组 · 来源：风控参数基线与熔断规则 v1.0</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl param-tbl">
                <thead>
                  <tr>
                    <th>参数</th>
                    <th>范围</th>
                    <th className="is-num">阈值</th>
                    <th className="is-num">当前</th>
                    <th className="is-num">占用</th>
                    <th>级别</th>
                  </tr>
                </thead>
                <tbody>
                  {d.riskGroups.map((g) => (
                    <Fragment key={g.name}>
                      <tr className="group-row">
                        <td colSpan={6}>
                          {g.name}
                          <span>{g.source}</span>
                        </td>
                      </tr>
                      {g.items.map((p, i) => (
                        <tr key={`${g.name}-${p.key}-${i}`}>
                          <td>
                            <div className="param-key">
                              <b>{p.key}</b>
                              <span>{p.v}</span>
                            </div>
                          </td>
                          <td>
                            <span className="tag-mini">{p.scope}</span>
                          </td>
                          <td className="is-num">
                            {num(p.limit, p.unit)} {p.unit === "USDC" ? "" : p.unit}
                          </td>
                          <td className="is-num">
                            {p.cur === null ? (
                              <span className="faint">—</span>
                            ) : (
                              num(p.cur, p.unit) + (p.unit === "USDC" ? "" : " " + p.unit)
                            )}
                          </td>
                          <td>{usageCell(p)}</td>
                          <td>
                            <Chip variant={sevVariant(p.sev)}>{p.sev}</Chip>
                          </td>
                        </tr>
                      ))}
                    </Fragment>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">
            覆盖规则不得突破全局硬限制；参数变更新增版本记录与生效时间。未启用的参数（如实盘单笔上限）在 Paper
            阶段不参与计算。
          </div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>
              熔断规则 <span className="count">{d.circuitRules.length}</span>
            </h2>
            <div className="panel-meta">触发 → 范围 → 恢复条件 · §10</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>触发条件</th>
                    <th>熔断范围</th>
                    <th>恢复条件</th>
                    <th className="is-num">最近触发</th>
                  </tr>
                </thead>
                <tbody>
                  {d.circuitRules.map((r) => (
                    <tr key={r.trigger}>
                      <td className="cell-strong">{r.trigger}</td>
                      <td>{r.scope}</td>
                      <td className="muted" style={{ whiteSpace: "normal" }}>{r.recover}</td>
                      <td className={`is-num ${r.last !== "—" ? "warn" : "faint"}`}>{r.last}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">
            恢复必须记录：熔断原因、触发时间、影响范围、相关交易、账本状态、修复措施、恢复时间与恢复操作者。
          </div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>事件日志</h2>
            <div className="panel-meta">最近 24 小时</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>事件</th>
                    <th>时间</th>
                    <th>级别</th>
                    <th>范围</th>
                    <th>说明</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {d.riskEvents.map((e) => (
                    <tr key={e.id}>
                      <td className="cell-id">{e.id}</td>
                      <td className="mono muted" style={{ fontSize: 10.5 }}>{e.ts}</td>
                      <td>
                        <Chip variant={levelVariant(e.level)}>{e.level}</Chip>
                      </td>
                      <td>{e.scope}</td>
                      <td style={{ whiteSpace: "normal", maxWidth: 520, color: "var(--ink-3)" }}>{e.msg}</td>
                      <td>
                        <Chip variant={e.state === "OPEN" ? "bad" : "ok"} dot>
                          {e.state}
                        </Chip>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </section>
      </div>
    </>
  );
}
