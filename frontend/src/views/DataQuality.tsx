import { data } from "../data";
import type { Instrument, Opportunity } from "../types";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { PriceSourcePanel } from "../components/PriceSourcePanel";
import { toast } from "../ui/Toast";

/**
 * The legacy dataset also carries the data-quality feeds (TDD §3.3), but
 * DataModel in src/types.ts does not declare them yet. Keep that slice local
 * until the Axum API owns the shape; everything else comes from DataModel.
 */
interface RpcNode {
  chain: string;
  role: string;
  endpoint: string;
  block: number;
  hash: string;
  p95: number;
  err: number;
  state: string;
}

interface CexIntegrity {
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

interface ClockSkew {
  pair: string;
  skew: number;
  limit: number;
}

interface DataGap {
  at: string;
  dur: string;
  source: string;
  impact: string;
}

interface ArchiveCoverage {
  chain: string;
  from: string;
  to: string;
  coverage: number;
  readable: boolean;
  note: string;
}

interface DataQualityModel {
  instruments: Instrument[];
  opps: Opportunity[];
  rpcNodes: RpcNode[];
  cexIntegrity: CexIntegrity[];
  clockSkew: ClockSkew[];
  dataGaps: DataGap[];
  archiveCoverage: ArchiveCoverage[];
}

const d = data as unknown as DataQualityModel;

const INTEGRITY_FALLBACK: Pick<CexIntegrity, "gaps" | "reconnect" | "latency" | "lastSync"> = {
  gaps: 0,
  reconnect: 1,
  latency: 38,
  lastSync: "16:49:31",
};

export function DataQuality() {
  const chainCount = new Set(d.rpcNodes.map((n) => n.chain)).size;
  const divergedChains = new Set(d.rpcNodes.filter((n) => n.state !== "UP").map((n) => n.chain)).size;
  const validBooks = d.instruments.filter((i) => i.integrity === "VALID").length;
  const readable = d.archiveCoverage.filter((a) => a.readable).length;
  const localSkew = d.clockSkew[0];
  const exchangeSkew = d.clockSkew[1];
  const skewPct = (localSkew.skew / localSkew.limit) * 100;

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow"><span className="idx">09</span> RESEARCH · DATA QUALITY</div>
          <h1>数据质量</h1>
          <p className="page-sub">所有市场数据保留来源、时间戳与质量标记。数据不完整、盘口不完整或 RPC 状态不一致时，系统默认拒绝生成可执行机会。</p>
        </div>
        <div className="page-actions">
          <Chip variant="warn" dot>{divergedChains} DIVERGED</Chip>
          <button
            className="btn"
            onClick={() =>
              toast("数据质量校验完成", {
                sub: "1 条链不一致 · 1 个盘口失效 · 1 段归档不可读",
                variant: "warn",
              })
            }
          >
            <Icon name="refresh" /> 重新校验
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">双 RPC 一致<span className="unit">CHAINS</span></div>
            <div className="kpi-value">{chainCount - divergedChains}<small>/ {chainCount}</small></div>
            <div className="kpi-foot"><span className="warn">Polygon 主备区块差 2 · Hash 不一致</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">盘口完整<span className="unit">BOOKS</span></div>
            <div className="kpi-value">{validBooks}<small>/ {d.instruments.length}</small></div>
            <div className="kpi-foot"><span className="neg">WBTC / POL 盘口失效</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">归档可读<span className="unit">ARCHIVE</span></div>
            <div className="kpi-value">{readable}<small>/ {d.archiveCoverage.length}</small></div>
            <div className="kpi-foot"><span>Ethereum 2 个区间不可读，已降级</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">时钟偏差<span className="unit">LOCAL ↔ EXCHANGE</span></div>
            <div className="kpi-value">{localSkew.skew}<small>ms</small></div>
            <div className="kpi-foot">
              <span>阈值 {localSkew.limit} ms · P95 {exchangeSkew.skew} ms</span>
              <span className="kpi-spark" style={{ width: 64 }}>
                <span className="bar"><i style={{ width: `${skewPct}%` }} /></span>
              </span>
            </div>
          </div>
        </div>

        <PriceSourcePanel />

        <section className="panel">
          <header className="panel-head">
            <h2>RPC 节点 <span className="count">{d.rpcNodes.length}</span></h2>
            <div className="panel-meta">主节点 + 备用节点 · 高度与 Hash 必须一致</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>链</th><th>角色</th><th>端点</th><th className="is-num">区块</th><th>Hash</th>
                    <th className="is-num">P95</th><th className="is-num">错误率</th><th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {d.rpcNodes.map((n) => (
                    <tr key={`${n.chain}-${n.role}`}>
                      <td className="cell-strong">{n.chain}</td>
                      <td><span className="tag-mini">{n.role}</span></td>
                      <td className="cell-id">{n.endpoint}</td>
                      <td className="is-num">{fmt.num(n.block, 0)}</td>
                      <td className="cell-id">{n.hash}</td>
                      <td className={`is-num ${n.p95 > 500 ? "warn" : ""}`}>{n.p95} ms</td>
                      <td className={`is-num ${n.err > 0.2 ? "warn" : "muted"}`}>{fmt.pct(n.err, 2)}</td>
                      <td><Chip variant={n.state === "UP" ? "ok" : "bad"} dot>{n.state}</Chip></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">Polygon 主备节点区块差 2 且 Hash 不一致 → 该链暂停扫描，恢复前不生成任何可执行机会。</div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>行情订阅与盘口完整性 <span className="count">{d.instruments.length}</span></h2>
            <div className="panel-meta">Snapshot ID · Sequence · 缺口 · 本地接收时间</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>交易对</th><th className="is-num">序列号</th><th className="is-num">缺口</th>
                    <th className="is-num">重连</th><th className="is-num">盘口年龄</th><th className="is-num">数据延迟</th>
                    <th className="is-num">最近完整同步</th><th className="is-num">机会 / 可执行</th><th>完整性</th><th>执行</th>
                  </tr>
                </thead>
                <tbody>
                  {d.instruments.map((i) => {
                    const b = d.cexIntegrity.find((x) => x.symbol === i.symbol) ?? INTEGRITY_FALLBACK;
                    const os = d.opps.filter((o) => o.symbol === i.symbol);
                    const act = os.filter((o) => o.status === "ACTIONABLE").length;
                    return (
                      <tr key={i.symbol}>
                        <td className="cell-strong">{i.symbol}<span className="cell-sub">{i.exchange}</span></td>
                        <td className="is-num muted">{fmt.num(i.seq, 0)}</td>
                        <td className={`is-num ${b.gaps ? "neg" : "muted"}`}>{b.gaps}</td>
                        <td className="is-num muted">{b.reconnect}</td>
                        <td className={`is-num ${i.age > 250 ? "neg" : ""}`}>{fmt.age(i.age)}</td>
                        <td className="is-num muted">{b.latency} ms</td>
                        <td className="is-num muted">{b.lastSync}</td>
                        <td className={`is-num ${act ? "pos" : "muted"}`}>{os.length} / {act}</td>
                        <td><Chip variant={i.integrity === "VALID" ? "ok" : "bad"} dot>{i.integrity}</Chip></td>
                        <td>
                          {i.tradable
                            ? i.integrity === "VALID" && i.age <= 250
                              ? <Chip variant="accent">可执行</Chip>
                              : <Chip variant="warn">暂停</Chip>
                            : <Chip variant="idle">仅行情</Chip>}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">发现缺口、乱序或断线时盘口立即标记 INVALID，必须重新拉取 Snapshot 后才恢复；只有完整且未超时的盘口才能生成可执行机会。</div>
        </section>

        <div className="grid g-even">
          <section className="panel">
            <header className="panel-head"><h2>时钟偏差</h2><div className="panel-meta">阈值 250 ms</div></header>
            <div className="panel-body">
              <div className="stack" style={{ gap: 12 }}>
                {d.clockSkew.map((c) => (
                  <div className="meter-row" key={c.pair}>
                    <span className="meter-label">{c.pair}</span>
                    <span className="meter-val">{c.skew} / {c.limit} ms</span>
                    <div className={`bar ${c.skew / c.limit > 0.8 ? "bar--warn" : ""}`}>
                      <i style={{ width: `${(c.skew / c.limit) * 100}%` }} />
                    </div>
                  </div>
                ))}
              </div>
            </div>
            <div className="panel-note">本机休眠、断网或系统更新后必须停止订单状态推进，并将行情标记为断档。</div>
          </section>

          <section className="panel">
            <header className="panel-head"><h2>断档记录</h2><div className="panel-meta">最近 24h</div></header>
            <div className="panel-body panel-body--flush">
              <div className="ev-list">
                {d.dataGaps.map((g) => (
                  <div className="ev" key={`${g.at}-${g.source}`}>
                    <div className="ev-top">
                      <span className="ev-scope">{g.source}</span>
                      <span className="chip chip--idle">{g.dur}</span>
                      <span className="ev-time">{g.at}</span>
                    </div>
                    <div className="ev-msg">{g.impact}</div>
                  </div>
                ))}
              </div>
            </div>
          </section>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>归档覆盖</h2>
            <div className="panel-meta">历史审计可用区间</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead><tr><th>链</th><th>区间</th><th className="is-num">覆盖率</th><th>归档可读</th><th>说明</th></tr></thead>
                <tbody>
                  {d.archiveCoverage.map((a) => (
                    <tr key={a.chain}>
                      <td className="cell-strong">{a.chain}</td>
                      <td className="mono" style={{ fontSize: 10.5 }}>{a.from} → {a.to}</td>
                      <td>
                        <div className="coverage">
                          <span className={`bar ${a.coverage >= 95 ? "" : a.coverage >= 85 ? "bar--warn" : "bar--bad"}`}>
                            <i style={{ width: `${a.coverage}%` }} />
                          </span>
                          <span className="mono" style={{ fontSize: 10, color: "var(--ink-2)" }}>{fmt.pct(a.coverage, 1)}</span>
                        </div>
                      </td>
                      <td><Chip variant={a.readable ? "ok" : "bad"}>{a.readable ? "可读" : "部分不可读"}</Chip></td>
                      <td className="muted" style={{ whiteSpace: "normal" }}>{a.note}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">归档 RPC 不可访问时，系统只能对可用区间做审计，不编造缺失历史状态；覆盖率不足的区间只能用于初筛。</div>
        </section>
      </div>
    </>
  );
}
