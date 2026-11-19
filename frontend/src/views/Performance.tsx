import { useState } from "react";
import { data } from "../data";
import { fmt, signClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { LineChart } from "../charts/LineChart";
import { toast } from "../ui/Toast";

const RANGES = { "7D": 7, "30D": 30, "ALL": 90 } as const;
type RangeKey = keyof typeof RANGES;
const RANGE_KEYS: RangeKey[] = ["7D", "30D", "ALL"];

interface StrategyRow {
  key: string;
  label: string;
  trades: number;
  net: number;
  win: number;
  avg: number;
}

const STRATEGIES: StrategyRow[] = [
  { key: "dex_triple", label: "DEX 三角套利", trades: 96, net: 84.2, win: 71.9, avg: 0.88 },
  { key: "dex_cross", label: "DEX 跨池套利", trades: 64, net: 41.35, win: 64.1, avg: 0.65 },
  { key: "cex_dex", label: "CEX–DEX 对冲", trades: 54, net: 57.08, win: 61.1, avg: 1.06 },
];

const NET_TOTAL = 182.63;

interface Bin {
  from: number;
  to: number;
  count: number;
  mid: number;
  tone: "pos" | "neg";
}

const BINS: Bin[] = (
  [
    { from: -3, to: -2, count: 4 },
    { from: -2, to: -1, count: 9 },
    { from: -1, to: 0, count: 38 },
    { from: 0, to: 1, count: 62 },
    { from: 1, to: 2, count: 51 },
    { from: 2, to: 3, count: 32 },
    { from: 3, to: 4, count: 14 },
    { from: 4, to: 5, count: 4 },
  ] as { from: number; to: number; count: number }[]
).map<Bin>((b) => ({ ...b, mid: (b.from + b.to) / 2, tone: b.to <= 0 ? "neg" : "pos" }));

const BIN_TOTAL = BINS.reduce((s, b) => s + b.count, 0);
const POS_COUNT = BINS.filter((b) => b.from >= 0).reduce((s, b) => s + b.count, 0);
const BIN_MAX = Math.max(...BINS.map((b) => b.count)) || 1;
const BIN_ZERO = BINS.findIndex((b) => b.from === 0);

/** 原型 histogram 的数值格式：负区间保留符号，正区间加 + */
const binLabel = (v: number) => (v > 0 ? "+" : "") + v.toFixed(0);

const OPS: [string, string, string][] = [
  ["RPC P95", "188 ms", "主备双节点"],
  ["RPC 错误率", "0.02%", "24h 请求"],
  ["CEX WS 延迟", "38 ms", "Binance"],
  ["盘口完整率", "99.4%", "三交易对"],
  ["数据年龄 P95", "96 ms", "阈值 250 ms"],
  ["扫描延迟", "42 ms", "报价 → 机会"],
  ["模拟耗时", "18 ms", "REVM 单次"],
  ["机会到期率", "31.2%", "未成交即过期"],
  ["Paper 成交率", "25.0%", "双腿完成"],
  ["对冲完成率", "25.0%", "含部分成交"],
  ["部分成交率", "18.4%", "IOC"],
  ["DEX 回滚率", "6.1%", "含 Gas 损失"],
  ["平均补偿成本", "0.34", "USDC / 笔"],
  ["最大未对冲敞口", "18.20", "USDC"],
  ["账本差异", "0.00", "对账通过"],
];

export function Performance() {
  const [range, setRange] = useState<RangeKey>("30D");

  const n = RANGES[range];
  const sv = data.series.slice(-n);
  const first = sv[0].equity;
  const last = sv[sv.length - 1].equity;
  const pnl = last - first;
  const ret = (pnl / first) * 100;
  const ann = (Math.pow(1 + ret / 100, 365 / n) - 1) * 100;

  let peak = -Infinity;
  let ddUsd = 0;
  const ddSeries = sv.map((p) => {
    peak = Math.max(peak, p.equity);
    const gap = peak - p.equity;
    ddUsd = Math.max(ddUsd, gap);
    return gap / peak;
  });
  const dd = Math.max(...ddSeries) * 100;

  const mid = sv[Math.floor(sv.length / 2)];
  const dayLabel = (i: number) => `${sv[i].date.getMonth() + 1}月${sv[i].date.getDate()}日`;

  const rangeLabel = range === "ALL" ? "90D" : range;
  const netTotal = STRATEGIES.reduce((s, x) => s + x.net, 0);

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">06</span> PAPER · PERFORMANCE
          </div>
          <h1>绩效</h1>
          <p className="page-sub">
            收益、风险与容量同时呈现。年化 100% 为挑战目标，不构成验收承诺；当前折算仅代表 90 个交易日的模拟结果。
          </p>
        </div>
        <div className="page-actions">
          <div className="seg" role="group" aria-label="统计区间">
            {RANGE_KEYS.map((k) => (
              <button
                key={k}
                className={k === range ? "is-active" : ""}
                onClick={() => setRange(k)}
              >
                {k === "ALL" ? "90D" : k}
              </button>
            ))}
          </div>
          <button
            className="btn"
            onClick={() => toast("绩效报告已生成", { sub: "含逐笔成本明细与情景统计 · CSV" })}
          >
            <Icon name="external" /> 导出报告
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">
              区间净收益<span className="unit">{rangeLabel}</span>
            </div>
            <div className={`kpi-value ${signClass(pnl)}`}>{fmt.signed(pnl)}</div>
            <div className="kpi-foot">
              <span className={`delta ${signClass(pnl)}`}>{fmt.signed(ret)}%</span>
              <span>· {sv.length} 个交易日</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              年化折算<span className="unit">ANNUALIZED</span>
            </div>
            <div className={`kpi-value ${ann >= 0 ? "pos" : "neg"}`}>{fmt.signed(ann, 1)}%</div>
            <div className="kpi-foot">
              <span>目标 100% · 不构成承诺</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              收益风险比<span className="unit">SHARPE</span>
            </div>
            <div className="kpi-value">2.31</div>
            <div className="kpi-foot">
              <span>无风险利率 4.0% · 日频</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              最大回撤<span className="unit">PEAK → TROUGH</span>
            </div>
            <div className="kpi-value">{fmt.pct(dd)}</div>
            <div className="kpi-foot">
              <span className="delta neg">{fmt.signed(-ddUsd)}</span>
              <span>USDC · 上限 5.00%</span>
            </div>
          </div>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>
              权益与回撤 <span className="count">{rangeLabel}</span>
            </h2>
            <div className="panel-meta">
              <span className="legend">
                <span>
                  <i />
                  权益
                </span>
                <span>
                  <i className="is-dash" />
                  基准 5,000
                </span>
              </span>
            </div>
          </header>
          <div className="panel-body">
            <LineChart
              values={sv.map((p) => p.equity)}
              height={220}
              baseline={5000}
              tone="accent"
              yFormat={(v) => fmt.num(v, 0)}
              hoverFormat={(v) => fmt.num(v, 2) + " USDC"}
              hoverLabel={(i) => dayLabel(i)}
              xLabels={[
                { text: `${sv[0].date.getMonth() + 1}/${sv[0].date.getDate()}`, at: 1 },
                { text: `${mid.date.getMonth() + 1}/${mid.date.getDate()}`, at: 50 },
                {
                  text: `${sv[sv.length - 1].date.getMonth() + 1}/${sv[sv.length - 1].date.getDate()}`,
                  at: 99,
                },
              ]}
            />
          </div>
          <div className="panel-body" style={{ borderTop: "1px solid var(--line-1)" }}>
            <div className="row between mb-8">
              <span className="eyebrow">DRAWDOWN</span>
              <span className="mono" style={{ fontSize: 10, color: "var(--neg)" }}>
                最深 {fmt.pct(dd)}
              </span>
            </div>
            <LineChart
              values={ddSeries.map((v) => -v * 100)}
              height={92}
              tone="neg"
              baseline={0}
              fillToBaseline
              yFormat={(v) => v.toFixed(1) + "%"}
              hoverFormat={(v) => v.toFixed(2) + "%"}
              hoverLabel={(i) => dayLabel(i)}
              xLabels={[]}
            />
          </div>
        </section>

        <div className="grid g-main">
          <section className="panel">
            <header className="panel-head">
              <h2>策略分解</h2>
              <div className="panel-meta">净利合计 {fmt.num(netTotal)}</div>
            </header>
            <div className="panel-body panel-body--flush">
              <div className="tbl-wrap">
                <table className="tbl">
                  <thead>
                    <tr>
                      <th>策略</th>
                      <th className="is-num">交易</th>
                      <th className="is-num">净利</th>
                      <th className="is-num">胜率</th>
                      <th className="is-num">单笔均值</th>
                      <th>贡献</th>
                    </tr>
                  </thead>
                  <tbody>
                    {STRATEGIES.map((s) => (
                      <tr key={s.key}>
                        <td className="cell-strong">
                          {s.label}
                          <span className="cell-sub">{s.key}</span>
                        </td>
                        <td className="is-num">{s.trades}</td>
                        <td className={`is-num ${signClass(s.net)}`}>{fmt.signed(s.net)}</td>
                        <td className="is-num">{fmt.pct(s.win, 1)}</td>
                        <td className={`is-num ${signClass(s.avg)}`}>{fmt.signed(s.avg)}</td>
                        <td>
                          <div className="usage-cell" style={{ gridTemplateColumns: "90px 44px" }}>
                            <span className="bar">
                              <i style={{ width: `${(s.net / 84.2) * 100}%` }} />
                            </span>
                            <span className="usage-num">{fmt.pct((s.net / NET_TOTAL) * 100, 0)}</span>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
            <div className="panel-note">逐笔净利润可下钻至成本构成；理论价差与闪电贷本金不计入收益。</div>
          </section>

          <section className="panel">
            <header className="panel-head">
              <h2>关键指标</h2>
              <div className="panel-meta">214 笔交易</div>
            </header>
            <div className="panel-body">
              <dl className="kv">
                <dt>成交率（双腿）</dt>
                <dd>66.7%</dd>
                <dt>DEX 回滚率</dt>
                <dd>6.1%</dd>
                <dt>CEX 部分成交率</dt>
                <dd>18.4%</dd>
                <dt>平均持仓 / 敞口</dt>
                <dd>0.98 s</dd>
                <dt>资金利用率</dt>
                <dd>62.0%</dd>
                <dt>最差连续亏损</dt>
                <dd className="neg">5 笔 · −6.42</dd>
                <dt>日均净利</dt>
                <dd>2.03</dd>
              </dl>
            </div>
          </section>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>
              单笔净利分布{" "}
              <span className="count">
                {BIN_TOTAL} 笔 · 正 {POS_COUNT} / 负 {BIN_TOTAL - POS_COUNT}
              </span>
            </h2>
            <div className="panel-meta">USDC · 按 1 USDC 分箱</div>
          </header>
          <div className="panel-body">
            <div className="hist" style={{ ["--hist-h" as string]: "150px" }}>
              {BIN_ZERO > 0 && (
                <b
                  className="hist-zero"
                  style={{ left: `${((BIN_ZERO / BINS.length) * 100).toFixed(2)}%` }}
                />
              )}
              {BINS.map((b, i) => (
                <div
                  className="hist-col"
                  key={i}
                  title={`${binLabel(b.from)} – ${binLabel(b.to)}： ${b.count}`}
                >
                  <i
                    className={`hist-bar hist-bar--${b.tone}`}
                    style={{ height: `${Math.max(2, (b.count / BIN_MAX) * 100).toFixed(1)}%` }}
                  />
                </div>
              ))}
            </div>
            <div className="hist-x">
              {BINS.map((b, i) =>
                i % 3 === 0 ? (
                  <span
                    key={i}
                    style={{ left: `${((i / BINS.length) * 100).toFixed(2)}%` }}
                  >
                    {binLabel(b.from)}
                  </span>
                ) : null,
              )}
            </div>
          </div>
          <div className="panel-note">分布右偏：多数交易净利 0–2 USDC，左尾来自回滚 Gas 与对冲补偿。</div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>
              运行指标 <span className="count">TDD §15</span>
            </h2>
            <div className="panel-meta">最近 24 小时 · 与收益分开评估</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="stat-grid stat-grid--5">
              {OPS.map(([k, v, note]) => (
                <div className="stat" key={k}>
                  <span>{k}</span>
                  <b>{v}</b>
                  <small>{note}</small>
                </div>
              ))}
            </div>
          </div>
          <div className="panel-note">
            数据年龄、账本差异或盘口完整率异常时，机会质量必须先降级并暂停相关策略，不能只看收益曲线。
          </div>
        </section>

        <div className="note">
          缺少历史 L2 深度、准确费率或完整区块前状态的样本已降低审计等级；本页数据来自 Paper 环境，不得直接推断为实盘收益。
        </div>
      </div>
    </>
  );
}
