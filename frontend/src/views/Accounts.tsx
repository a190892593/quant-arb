import { data } from "../data";
import { fmt, signClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

/** inventory model — the prototype hard-codes these venue splits */
interface InventoryRow {
  sym: string;
  dexArb: number;
  dexBase: number;
  cex: number;
  total: number;
  value: number;
  frozen: number;
  available: number;
  pending: number;
}

const VENUES = data.venues;

const INVENTORY: InventoryRow[] = [
  { sym: "WETH", dexArb: 0.412, dexBase: 0.1135, cex: 0.1148 },
  { sym: "USDC", dexArb: 1842.6, dexBase: 320.4, cex: 0 },
  { sym: "USDT", dexArb: 0, dexBase: 0, cex: 1231.2 },
].map((r) => {
  const totalQty = r.dexArb + r.dexBase + r.cex;
  const value = r.sym === "WETH" ? totalQty * data.px.WETH : r.sym === "USDT" ? totalQty * data.px.USDT : totalQty;
  const frozen = r.sym === "USDC" ? 120.4 : 0;
  return { ...r, total: totalQty, value, frozen, available: totalQty - frozen, pending: 0 };
});

export function Accounts() {
  const d = data;
  const total = d.equity;
  const dexValue = VENUES.filter((v) => v.kind === "DEX").reduce((s, v) => s + v.value, 0);
  const cexValue = VENUES.filter((v) => v.kind === "CEX").reduce((s, v) => s + v.value, 0);
  const dexPct = (dexValue / total) * 100;
  const ret = ((total - d.startEquity) / d.startEquity) * 100;

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">05</span> PAPER · ACCOUNTS
          </div>
          <h1>账户与库存</h1>
          <p className="page-sub">
            CEX 余额、链上钱包余额与闪电贷额度分开记账，不重复计入可用资金。跨场所库存偏移单独计量并生成再平衡任务。
          </p>
        </div>
        <div className="page-actions">
          <button
            className="btn"
            onClick={() => toast("库存快照已生成", { sub: "18 个资产行 · 写入 inventory_snapshots" })}
          >
            <Icon name="refresh" /> 生成库存快照
          </button>
          <button
            className="btn"
            onClick={() =>
              toast("再平衡任务草稿已创建", { sub: "需人工确认路径与成本后再执行", variant: "warn" })
            }
          >
            <Icon name="route" /> 新建再平衡
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">
              总权益<span className="unit">USDC</span>
            </div>
            <div className="kpi-value">{fmt.num(total)}</div>
            <div className="kpi-foot">
              <span className={`delta ${signClass(ret)}`}>{fmt.signed(ret)}%</span>
              <span>相对 5,000 基准</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              可用资金<span className="unit">AVAILABLE</span>
            </div>
            <div className="kpi-value">{fmt.num(total - 120.4)}</div>
            <div className="kpi-foot">
              <span>冻结 120.40 · 待结算 0.00</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              资金分布<span className="unit">DEX : CEX</span>
            </div>
            <div className="kpi-value">
              {fmt.num(dexPct, 1)}
              <small>%</small>
            </div>
            <div className="kpi-foot" style={{ flexWrap: "wrap" }}>
              <span className="kpi-spark" style={{ width: "100%", order: 2 }}>
                <span className="bar">
                  <i style={{ width: `${dexPct}%` }} />
                </span>
              </span>
              <span style={{ order: 1 }}>
                DEX {fmt.num(dexValue)} · CEX {fmt.num(cexValue)}
              </span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              跨场所净敞口<span className="unit">IMBALANCE</span>
            </div>
            <div className="kpi-value">{fmt.num(164.2)}</div>
            <div className="kpi-foot">
              <span>WETH 偏移 · 上限 500.00</span>
              <span className="kpi-spark" style={{ width: 64 }}>
                <span className="bar">
                  <i style={{ width: "32.8%" }} />
                </span>
              </span>
            </div>
          </div>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>
              场所账户 <span className="count">{VENUES.length}</span>
            </h2>
            <div className="panel-meta">独立账本 · 不跨场所合并可用余额</div>
          </header>
          <div className="panel-body">
            <div className="venue-grid">
              {VENUES.map((v) => (
                <div className="venue" key={v.id}>
                  <div className="venue-head">
                    <div>
                      <div className="venue-kind">
                        {v.kind} · {v.chain ?? v.exchange ?? "CLEARING"}
                      </div>
                      <div className="venue-name">{v.name}</div>
                    </div>
                    <Chip variant={v.state === "PAPER_READY" ? "ok" : v.state === "PAPER" ? "warn" : "idle"}>
                      {v.state}
                    </Chip>
                  </div>
                  <div className="venue-value">
                    {fmt.num(v.value)}
                    <small>USDC</small>
                  </div>
                  <div className="venue-assets">
                    {v.assets.map((a) => (
                      <div className="venue-asset" key={a.sym}>
                        <span>{a.sym}</span>
                        <b>{a.qty === 0 ? "0.00" : fmt.num(a.qty, a.sym === "WETH" || a.sym === "ETH" ? 4 : 2)}</b>
                      </div>
                    ))}
                  </div>
                  <div className="hint" style={{ fontFamily: "var(--font-mono)", fontSize: 9 }}>
                    {v.address ?? "内部清算账户"} · 冻结 0.00
                  </div>
                </div>
              ))}
            </div>
          </div>
          <div className="panel-note">
            资金分配当前为 DEX 70% / CEX 30%；50:50 仅作为第一阶段对比测试基准，未认定为最优配置。
          </div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>库存明细</h2>
            <div className="panel-meta">按场所隔离 · 合并净持仓 · 估值为 USDC</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl inv-tbl">
                <thead>
                  <tr>
                    <th>资产</th>
                    <th className="is-num">DEX · ARB</th>
                    <th className="is-num">DEX · BASE</th>
                    <th className="is-num">CEX · BINANCE</th>
                    <th className="is-num">合并净持仓</th>
                    <th className="is-num">可用</th>
                    <th className="is-num">冻结</th>
                    <th className="is-num">待结算</th>
                    <th className="is-num">估值</th>
                    <th>场所偏移</th>
                  </tr>
                </thead>
                <tbody>
                  {INVENTORY.map((r) => {
                    const dp = r.sym === "WETH" ? 4 : 2;
                    const shift = r.sym === "WETH" ? 30.4 : 0;
                    return (
                      <tr key={r.sym}>
                        <td className="cell-strong">{r.sym}</td>
                        <td className="is-num">{fmt.num(r.dexArb, dp)}</td>
                        <td className="is-num">{fmt.num(r.dexBase, dp)}</td>
                        <td className="is-num">{fmt.num(r.cex, dp)}</td>
                        <td className="is-num cell-strong">{fmt.num(r.total, dp)}</td>
                        <td className="is-num">{fmt.num(r.available, dp)}</td>
                        <td className={`is-num ${r.frozen > 0 ? "warn" : "muted"}`}>{fmt.num(r.frozen, dp)}</td>
                        <td className="is-num muted">{fmt.num(r.pending, dp)}</td>
                        <td className="is-num">{fmt.num(r.value)}</td>
                        <td>
                          <div className="usage-cell" style={{ gridTemplateColumns: "74px 44px" }}>
                            <span className={`bar ${shift > 20 ? "bar--warn" : ""}`}>
                              <i style={{ width: `${shift}%` }} />
                            </span>
                            <span className="usage-num">{shift ? fmt.num(shift, 1) + "%" : "—"}</span>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">
            合并净持仓用于风险计量，执行时必须使用对应场所的可用余额；链上充值、提现与跨链桥转账不计入原子套利路径。
          </div>
        </section>

        <div className="grid g-even">
          <section className="panel">
            <header className="panel-head">
              <h2>敞口</h2>
              <div className="panel-meta">单 Token 上限 500.00</div>
            </header>
            <div className="panel-body">
              <div className="stack" style={{ gap: 12 }}>
                <div className="meter-row">
                  <span className="meter-label">WETH</span>
                  <span className="meter-val">164.20 / 500.00</span>
                  <div className="bar">
                    <i style={{ width: "32.8%" }} />
                  </div>
                </div>
                <div className="meter-row">
                  <span className="meter-label">USDC</span>
                  <span className="meter-val">96.40 / 500.00</span>
                  <div className="bar">
                    <i style={{ width: "19.3%" }} />
                  </div>
                </div>
                <div className="meter-row">
                  <span className="meter-label">USDT</span>
                  <span className="meter-val">41.80 / 500.00</span>
                  <div className="bar">
                    <i style={{ width: "8.4%" }} />
                  </div>
                </div>
                <div className="meter-row">
                  <span className="meter-label">未对冲（实时）</span>
                  <span className="meter-val">3.40 / 25.00</span>
                  <div className="bar">
                    <i style={{ width: "13.6%" }} />
                  </div>
                </div>
              </div>
            </div>
          </section>

          <section className="panel">
            <header className="panel-head">
              <h2>再平衡任务</h2>
              <div className="panel-meta">费用单独核算</div>
            </header>
            <div className="panel-body panel-body--flush">
              <div className="tbl-wrap">
                <table className="tbl tbl-compact">
                  <thead>
                    <tr>
                      <th>任务</th>
                      <th>路径</th>
                      <th className="is-num">成本</th>
                      <th>状态</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td className="cell-id">RB-2211</td>
                      <td>
                        Base → Arbitrum · 350 USDC<span className="cell-sub">跨链桥 12 min</span>
                      </td>
                      <td className="is-num neg">0.62</td>
                      <td>
                        <Chip variant="ok">完成</Chip>
                      </td>
                    </tr>
                    <tr>
                      <td className="cell-id">RB-2212</td>
                      <td>
                        Arbitrum → Binance · 0.0500 WETH<span className="cell-sub">提币审核中</span>
                      </td>
                      <td className="is-num muted">1.40</td>
                      <td>
                        <Chip variant="warn">待执行</Chip>
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>
            <div className="panel-note">再平衡成本不得隐藏在策略利润中；本次统计 2.02 USDC 已单独入账。</div>
          </section>
        </div>
      </div>
    </>
  );
}
