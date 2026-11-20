import { data } from "../data";
import { fmt, signClass } from "../lib/format";
import { Spark } from "../charts/Spark";
import { LineChart } from "../charts/LineChart";
import { StatusChip, Chip } from "../ui/Chip";

const md = (p?: { date: Date }) =>
  p && !Number.isNaN(p.date.getTime()) ? `${p.date.getMonth() + 1}/${p.date.getDate()}` : "—";

const xLabels = (points: { date: Date }[]) => {
  if (points.length < 2) return [];
  const at = (i: number) => Math.round((i / (points.length - 1)) * 100);
  const last = points.length - 1;
  return [
    { text: md(points[0]), at: 1 },
    { text: md(points[Math.floor(last / 2)]), at: at(Math.floor(last / 2)) },
    { text: md(points[last]), at: 99 },
  ];
};

export function Overview() {
  const d = data;
  const equity = d.equity;
  const ret = ((equity - d.startEquity) / d.startEquity) * 100;
  const last30 = d.series.slice(-30);

  const rank: Record<string, number> = { ACTIONABLE: 0, WATCH: 1, REJECTED: 2, EXPIRED: 3, EXECUTED: 4 };
  const top = [...d.opps]
    .sort((a, b) => (rank[a.status] ?? 9) - (rank[b.status] ?? 9) || b.net - a.net)
    .slice(0, 5);

  const guards = [
    { label: "全局日亏损", cur: 12.4, lim: 50, pct: 24.8 },
    { label: "最大回撤", cur: d.maxDDusd, lim: 250, pct: (d.maxDDusd / 250) * 100 },
    { label: "场资金使用率", cur: 62, lim: 80, pct: 77.5 },
  ];

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">01</span> MONITOR · OVERVIEW
          </div>
          <h1>总览</h1>
          <p className="page-sub">{`本地 Paper 环境 · 权益基准 ${fmt.num(d.startEquity, 0)} USDC · ${d.meta?.online ? "数据来自本地后端（Paper 账本），不涉及真实资金" : "后端未连接，当前为离线演示数据"}`}</p>
        </div>
        <div className="page-actions">
          <div className="seg" role="group" aria-label="时间范围">
            <button className="is-active">30D</button>
            <button>7D</button>
            <button>今日</button>
          </div>
          <a className="btn" href="#/scanner-dex">打开扫描器</a>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">总权益<span className="unit">USDC</span></div>
            <div className="kpi-value">{fmt.num(equity)}</div>
            <div className="kpi-foot">
              <span className={`delta ${signClass(d.todayPnl)}`}>{fmt.signed(d.todayPnl)}</span>
              <span>今日</span>
              <span className="kpi-spark">
                <Spark values={d.series.slice(-24).map((p) => p.equity)} />
              </span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">区间净收益<span className="unit">SINCE 08-22</span></div>
            <div className={`kpi-value ${signClass(equity - d.startEquity)}`}>{fmt.signed(equity - d.startEquity)}</div>
            <div className="kpi-foot">
              <span className={`delta ${signClass(equity - d.startEquity)}`}>{fmt.signed(ret, 2)}%</span>
              <span>· 90 个交易日</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">最大回撤<span className="unit">PEAK → TROUGH</span></div>
            <div className="kpi-value">{fmt.pct(d.maxDD * 100)}</div>
            <div className="kpi-foot">
              <span className="delta neg">{fmt.signed(-d.maxDDusd)}</span>
              <span>USDC · 上限 5.00%</span>
            </div>
          </div>
        </div>

        <div className="hero">
          <section className="panel">
            <header className="panel-head">
              <h2>权益曲线 <span className="count">USDC · 30D</span></h2>
              <div className="panel-meta">
                <span className="legend">
                  <span><i />权益</span>
                  <span><i className="is-dash" />基准 5,000</span>
                </span>
              </div>
            </header>
            <div className="panel-body">
              <div className="hero-value">
                <b>{fmt.num(equity)}</b>
                <small>USDC</small>
                <span className="chip chip--ok">+{fmt.num(ret, 2)}% 累计</span>
              </div>
              <div className="hero-sub">
                <div><span>今日</span><b className={signClass(d.todayPnl)}>{fmt.signed(d.todayPnl)}</b></div>
                <div><span>近 7 日</span><b className={signClass(d.weekPnl)}>{fmt.signed(d.weekPnl)}</b></div>
                <div><span>已实现</span><b>{fmt.num(equity - d.startEquity - 3.2)}</b></div>
                <div><span>未实现</span><b className="pos">+3.20</b></div>
                <div><span>回撤</span><b className="neg">{fmt.signed(-d.maxDDusd)}</b></div>
              </div>
              <div style={{ marginTop: 16 }}>
                <LineChart
                  values={last30.map((p) => p.equity)}
                  height={218}
                  baseline={5000}
                  yFormat={(v) => fmt.num(v, 0)}
                  hoverFormat={(v) => `${fmt.num(v, 2)} USDC`}
                  hoverLabel={(i) => {
                    const p = last30[i];
                    return `${p.date.getMonth() + 1}月${p.date.getDate()}日 · 回撤 ${(p.dd * 100).toFixed(2)}%`;
                  }}
                  xLabels={xLabels(last30)}
                />
              </div>
            </div>
            <div className="panel-note">
              权益 = 各场所可用余额 + 未结算 + 未实现盈亏；不包含闪电贷本金。基准线为 5,000 USDC 审计起始资金。
            </div>
          </section>

          <section className="panel">
            <header className="panel-head">
              <h2>风险守卫</h2>
              <div className="panel-meta"><Chip variant="ok" dot>ARMED</Chip></div>
            </header>
            <div className="panel-body">
              <div className="stack" style={{ gap: 14 }}>
                {guards.map((g) => (
                  <div className="meter-row" key={g.label}>
                    <span className="meter-label">{g.label}</span>
                    <span className="meter-val">
                      {fmt.num(g.cur)} / {fmt.num(g.lim)}
                      {g.label.includes("使用率") ? " %" : ""}
                    </span>
                    <div className={`bar${g.pct >= 85 ? " bar--bad" : g.pct >= 60 ? " bar--warn" : ""}`}>
                      <i style={{ width: `${Math.min(100, g.pct)}%` }} />
                    </div>
                  </div>
                ))}
              </div>
            </div>
            <div className="panel-body panel-body--tight" style={{ borderTop: "1px solid var(--line-1)" }}>
              <div className="row between">
                <span className="hint">硬风控由后端强制执行，前端开关不可绕过。</span>
                <a className="btn btn--sm btn--ghost" href="#/risk">风控台</a>
              </div>
            </div>
          </section>
        </div>

        <div className="grid g-main">
          <section className="panel">
            <header className="panel-head">
              <h2>
                实时机会 <span className="count">{d.opps.filter((o) => o.status === "ACTIONABLE").length} ACTIONABLE / {d.opps.length}</span>
              </h2>
              <div className="panel-meta"><a className="btn btn--sm btn--ghost" href="#/scanner-dex">全部</a></div>
            </header>
            <div className="panel-body panel-body--flush">
              <div className="tbl-wrap">
                <table className="tbl tbl-compact">
                  <thead>
                    <tr>
                      <th>机会</th><th>策略</th><th>场所</th>
                      <th className="is-num">名义</th><th className="is-num">净利</th>
                      <th className="is-num">bps</th><th>风险</th><th>状态</th>
                    </tr>
                  </thead>
                  <tbody>
                    {top.map((o) => (
                      <tr key={o.id} className="is-link" onClick={() => { location.hash = `#/opportunity/${o.id}`; }}>
                        <td className="cell-id">{o.id}</td>
                        <td>{d.STRATEGY[o.strategy].label}</td>
                        <td className="cell-strong">
                          {o.cex ? `${o.cex} · ` : ""}{d.CHAIN[o.chain].label}
                          <span className="cell-sub">{o.pair}</span>
                        </td>
                        <td className="is-num">{fmt.num(o.notional, 0)}</td>
                        <td className={`is-num ${signClass(o.net)}`}>{fmt.signed(o.net)}</td>
                        <td className="is-num muted">{fmt.num(o.netBps, 1)}</td>
                        <td className={`is-num ${o.riskScore >= 70 ? "pos" : o.riskScore >= 50 ? "" : "neg"}`}>{o.riskScore}</td>
                        <td><StatusChip status={o.status} /></td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          </section>

          <div className="stack">
            <section className="panel">
              <header className="panel-head">
                <h2>扫描漏斗 <span className="count">近 5 分钟</span></h2>
                <div className="panel-meta"><Chip variant="ok" dot>运行中</Chip></div>
              </header>
              <div className="panel-body">
                <div className="funnel">
                  <div className="funnel-row"><span>检出</span><div className="funnel-bar funnel-bar--dim"><i style={{ width: "100%" }} /></div><b>{fmt.num(d.scanner.detected, 0)}</b></div>
                  <div className="funnel-row"><span>通过模拟</span><div className="funnel-bar funnel-bar--dim"><i style={{ width: "62%" }} /></div><b>{fmt.num(Math.max(0, d.scanner.detected - d.scanner.rejected), 0)}</b></div>
                  <div className="funnel-row"><span>风控通过</span><div className="funnel-bar"><i style={{ width: "11%" }} /></div><b>{fmt.num(d.scanner.actionable, 0)}</b></div>
                  <div className="funnel-row"><span>已拒绝</span><div className="funnel-bar funnel-bar--bad"><i style={{ width: "11%" }} /></div><b>{fmt.num(d.scanner.rejected, 0)}</b></div>
                </div>
              </div>
              <div className="panel-body panel-body--tight" style={{ borderTop: "1px solid var(--line-1)" }}>
                <div className="row between">
                  <span className="hint">{fmt.num(d.scanner.quotes, 0)} quotes/s · {d.scanner.pools} 池 · {d.scanner.chains} 链</span>
                  <span className="chip chip--idle">{d.scanner.uptime}</span>
                </div>
              </div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>数据源</h2>
                <div className="panel-meta"><Chip variant="ok" dot>HEALTHY</Chip></div>
              </header>
              <div className="panel-body panel-body--flush">
                <div className="health-grid">
                  {d.health.map((h) => (
                    <div className="health-cell" key={h.key}>
                      <div className="k">{h.key}<span className="dot dot--ok" /></div>
                      <div className="n">{h.note}</div>
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
