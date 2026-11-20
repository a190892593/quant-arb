import { useState } from "react";
import { data } from "../data";
import type { AuditJob } from "../types";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

type AuditVerdict = AuditJob["verdict"];

const jobs = data.auditJobs;

const VERDICT_VARIANT: Record<AuditVerdict, "ok" | "warn" | "bad" | "idle"> = {
  PASS: "ok",
  WATCH: "warn",
  REJECT: "bad",
  PAPER: "idle",
};

interface Check {
  name: string;
  ok: boolean;
  note: string;
}

function checksFor(j: AuditJob): Check[] {
  return [
    { name: "区块 / 交易顺序一致性", ok: true, note: "Block → TxIndex → LogIndex" },
    {
      name: "交易执行前状态可读",
      ok: j.gaps === 0,
      note: j.gaps ? j.gaps + " 个区间使用区块结束状态，已降级" : "全部可读取",
    },
    { name: "L2 盘口增量完整率", ok: j.coverage >= 95, note: fmt.pct(j.coverage, 1) + " 覆盖" },
    { name: "费率表版本匹配", ok: true, note: "cex_fee_schedules v3" },
    {
      name: "市场时钟偏差 ≤ 250 ms",
      ok: j.model !== "M1",
      note: j.model === "M1" ? "存在 412 ms 偏差区间" : "P95 96 ms",
    },
    {
      name: "样本外区间验证",
      ok: j.verdict === "PASS" || j.verdict === "WATCH",
      note: j.verdict === "REJECT" ? "剔除最优 3 日后净利转负" : "已完成",
    },
  ];
}

const MODEL_NOTE: Record<string, string> = {
  M3: "含压力与补偿",
  M2: "REVM 分叉复核",
};

const TIMELINE: readonly { title: string; note: string; ok: boolean }[] = [
  { title: "数据质量审查", note: "覆盖、缺口、时钟偏差", ok: true },
  { title: "训练区间参数估计", note: "按链 / 交易所 / 资金档位分组", ok: true },
  { title: "样本外验证", note: "禁止在同一区间反复调参", ok: true },
  { title: "Paper 前向验证", note: "进行中 · 需 ≥ 30 个交易日", ok: false },
  { title: "参数冻结与版本记录", note: "变更需记录原因与生效时间", ok: false },
];

export function Audit() {
  const [selected, setSelected] = useState<string>("AJ-2026-1113-02");

  const job = jobs.find((j) => j.id === selected) ?? jobs[0];
  const checks = checksFor(job);
  const counts: Record<AuditVerdict, number> = { PASS: 0, WATCH: 0, REJECT: 0, PAPER: 0 };
  jobs.forEach((j) => {
    counts[j.verdict] += 1;
  });

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow"><span className="idx">08</span> RESEARCH · AUDIT</div>
          <h1>历史审计</h1>
          <p className="page-sub">历史数据必须保留来源、覆盖区间、采样频率、时间戳、缺失率与可信等级。缺少 L2 深度的区间只能用于初筛。</p>
        </div>
        <div className="page-actions">
          <button className="btn" onClick={() => toast("审计列表已刷新")}>
            <Icon name="refresh" /> 刷新
          </button>
          <button
            className="btn btn--primary"
            onClick={() => toast("审计任务已排队", { sub: "arb-audit · 不会阻塞实时扫描", variant: "warn" })}
          >
            <Icon name="play" /> 新建审计任务
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">数据集<span className="unit">DATASETS</span></div>
            <div className="kpi-value">{jobs.length}</div>
            <div className="kpi-foot"><span>4 链 · 2 交易所 · L2 深覆盖</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">审计结论<span className="unit">VERDICT</span></div>
            <div className="kpi-value">{counts.PASS}<small>/ {jobs.length}</small></div>
            <div className="kpi-foot">
              <Chip variant="ok">PASS {counts.PASS}</Chip>
              <Chip variant="warn">WATCH {counts.WATCH}</Chip>
              <Chip variant="bad">REJECT {counts.REJECT}</Chip>
              <Chip variant="idle">PAPER {counts.PAPER}</Chip>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">样本总量<span className="unit">ROWS</span></div>
            <div className="kpi-value">{fmt.compact(jobs.reduce((s, j) => s + j.samples, 0))}</div>
            <div className="kpi-foot"><span>区块 / 盘口 / 逐笔成交</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">待复核<span className="unit">OPEN</span></div>
            <div className="kpi-value">2</div>
            <div className="kpi-foot"><span>Ethereum 归档缺口 · Binance L2 覆盖</span></div>
          </div>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>审计任务</h2>
            <div className="panel-meta">模型等级 M0 理论 → M3 压力</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>任务</th>
                    <th>区间</th>
                    <th>市场 / 协议</th>
                    <th className="is-num">样本</th>
                    <th>覆盖</th>
                    <th className="is-num">缺口</th>
                    <th>模型</th>
                    <th>结论</th>
                  </tr>
                </thead>
                <tbody>
                  {jobs.map((j) => (
                    <tr
                      key={j.id}
                      className={`is-link${j.id === job.id ? " is-selected" : ""}`}
                      tabIndex={0}
                      onClick={() => setSelected(j.id)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          setSelected(j.id);
                        }
                      }}
                    >
                      <td className="cell-id">{j.id}</td>
                      <td className="mono" style={{ fontSize: 10.5 }}>{j.range}</td>
                      <td className="cell-strong">{j.market}</td>
                      <td className="is-num">{fmt.compact(j.samples)}</td>
                      <td>
                        <div className="coverage">
                          <span
                            className={
                              "bar" +
                              (j.coverage >= 95 ? "" : j.coverage >= 85 ? " bar--warn" : " bar--bad")
                            }
                          >
                            <i style={{ width: `${j.coverage}%` }} />
                          </span>
                          <span className="mono" style={{ fontSize: 10, color: "var(--ink-2)" }}>{fmt.pct(j.coverage, 1)}</span>
                        </div>
                      </td>
                      <td className={`is-num ${j.gaps ? "warn" : "muted"}`}>{j.gaps}</td>
                      <td><span className="tag-mini">{j.model}</span></td>
                      <td>
                        <Chip variant={VERDICT_VARIANT[j.verdict]} dot>{j.verdict}</Chip>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </section>

        <div className="grid g-main">
          <section className="panel">
            <header className="panel-head">
              <h2>{job.id} <span className="count">{job.market}</span></h2>
              <div className="panel-meta">
                <Chip variant={VERDICT_VARIANT[job.verdict]} dot>{job.verdict}</Chip>
                <span>{job.model}</span>
              </div>
            </header>
            <div className="panel-body">
              <div className="price-ribbon" style={{ marginBottom: 14 }}>
                <div className="ribbon-cell"><span>覆盖区间</span><b style={{ fontSize: 13 }}>{job.range}</b><small>{job.market}</small></div>
                <div className="ribbon-cell"><span>覆盖率</span><b>{fmt.pct(job.coverage, 1)}</b><small>{job.gaps} 个缺口</small></div>
                <div className="ribbon-cell"><span>样本量</span><b>{fmt.compact(job.samples)}</b><small>行</small></div>
                <div className="ribbon-cell">
                  <span>模型等级</span>
                  <b>{job.model}</b>
                  <small>{MODEL_NOTE[job.model] ?? "仅初筛"}</small>
                </div>
              </div>
              <div className="gates">
                {checks.map((c) => (
                  <div className="gate" key={c.name}>
                    <span className={`gate-ico${c.ok ? "" : " is-warn"}`}>
                      {c.ok ? <Icon name="check" size={11} /> : <Icon name="alert" size={11} />}
                    </span>
                    <span className="gate-name">{c.name}<small>{c.note}</small></span>
                    <Chip variant={c.ok ? "ok" : "warn"}>{c.ok ? "PASS" : "WATCH"}</Chip>
                  </div>
                ))}
              </div>
            </div>
            <div className="panel-note">{job.note}</div>
          </section>

          <div className="stack">
            <section className="panel">
              <header className="panel-head"><h2>模型等级</h2></header>
              <div className="panel-body">
                <div className="stack" style={{ gap: 10 }}>
                  <div><div className="row between"><span className="mono" style={{ fontSize: 11, color: "var(--accent)" }}>M0</span><span className="hint">理论价差 · 不扣成本</span></div></div>
                  <div><div className="row between"><span className="mono" style={{ fontSize: 11, color: "var(--accent)" }}>M1</span><span className="hint">链上实时状态（V2 储备 / V3 slot0+liquidity，只读）</span></div></div>
                  <div><div className="row between"><span className="mono" style={{ fontSize: 11, color: "var(--accent)" }}>M2</span><span className="hint">REVM 分叉复核（QuoterV2，报价区块状态）</span></div></div>
                  <div><div className="row between"><span className="mono" style={{ fontSize: 11, color: "var(--accent)" }}>M3</span><span className="hint">压力：延迟 / 滑点 / 回滚 / 补偿</span></div></div>
                </div>
                <div className="divider" />
                <p className="hint">结论等级：PASS 证据充分且通过全部预设标准；WATCH 有可行性信号但数据或样本不足；REJECT 成本、容量或风险不符合要求；PAPER 仅在模拟环境保留观察。</p>
              </div>
            </section>

            <section className="panel">
              <header className="panel-head"><h2>参数校准流程</h2></header>
              <div className="panel-body">
                <div className="tl">
                  {TIMELINE.map((s) => (
                    <div className={`tl-item${s.ok ? " is-ok" : ""}`} key={s.title}>
                      <div className="tl-title">{s.title}</div>
                      <div className="tl-note">{s.note}</div>
                    </div>
                  ))}
                </div>
              </div>
            </section>
          </div>
        </div>
      </div>
    </>
  );
}
