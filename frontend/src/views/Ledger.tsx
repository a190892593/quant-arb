import { useState } from "react";
import { data } from "../data";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

type AccountFilter = "ALL" | "DEX" | "CEX" | "FEE";
type TypeFilter = "ALL" | "TRADE" | "FEE" | "GAS" | "FLASH_LOAN" | "PNL";

const ACCOUNT_FILTERS: { key: AccountFilter; label: string }[] = [
  { key: "ALL", label: "全部" },
  { key: "DEX", label: "DEX" },
  { key: "CEX", label: "CEX" },
  { key: "FEE", label: "费用" },
];

const TYPE_FILTERS: { key: TypeFilter; label: string }[] = [
  { key: "ALL", label: "全部" },
  { key: "TRADE", label: "成交" },
  { key: "FEE", label: "费用" },
  { key: "GAS", label: "Gas" },
  { key: "FLASH_LOAN", label: "闪电贷" },
  { key: "PNL", label: "盈亏" },
];

const FEES = [
  { label: "DEX 池费", v: 6.28 },
  { label: "Gas / Priority", v: 3.42 },
  { label: "CEX 手续费", v: 3.0 },
  { label: "库存再平衡", v: 1.02 },
  { label: "对冲补偿", v: 0.34 },
];
const FEE_TOTAL = FEES.reduce((s, f) => s + f.v, 0);
const GROSS_PROFIT = 22.72;

const STRATEGY_BOOK = [
  { label: "DEX 三角", trades: "96", fee: "−168.40", net: "+84.20" },
  { label: "DEX 跨池", trades: "64", fee: "−102.60", net: "+41.35" },
];

const RULES = [
  "流水不可覆盖，只允许追加与冲正",
  "每笔含来源、去向、交易前后余额与时间戳",
  "资产类流水按原始单位记账，不与稳定币混算",
  "失败回滚的 Gas 计入成本，不视为零成本",
  "再平衡费用单独入账，不隐藏在策略利润中",
];

function accountBucket(account: string): Exclude<AccountFilter, "ALL"> {
  if (account.indexOf("DEX") === 0) return "DEX";
  if (account.indexOf("CEX") === 0) return "CEX";
  return "FEE";
}

export function Ledger() {
  const [acct, setAcct] = useState<AccountFilter>("ALL");
  const [type, setType] = useState<TypeFilter>("ALL");

  const rows = data.ledger.filter((l) => {
    if (acct !== "ALL" && accountBucket(l.account) !== acct) return false;
    if (type !== "ALL" && l.type !== type) return false;
    return true;
  });

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">10</span> LEDGER · RECONCILIATION
          </div>
          <h1>账本</h1>
          <p className="page-sub">
            不可覆盖的流水记录：每一笔都包含来源、去向、交易前后余额、费用、时间戳与来源单据。前端累计盈亏必须与账本核对一致。
          </p>
        </div>
        <div className="page-actions">
          <button
            className="btn"
            onClick={() => toast("流水已导出", { sub: "ledger-entries-20261119.csv · 16 行" })}
          >
            <Icon name="external" /> 导出流水
          </button>
          <button
            className="btn"
            onClick={() => toast("对账完成：差异 0.00", { sub: "已核对 16 条流水 / 4 个账户" })}
          >
            <Icon name="check" /> 执行对账
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">
              总权益<span className="unit">LEDGER EQUITY</span>
            </div>
            <div className="kpi-value">{fmt.num(data.equity)}</div>
            <div className="kpi-foot">
              <span className="delta pos">{fmt.signed(data.equity - data.startEquity)}</span>
              <span>自 5,000 基准</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              今日已实现净利<span className="unit">REALIZED</span>
            </div>
            <div className="kpi-value">{fmt.num(8.66)}</div>
            <div className="kpi-foot">
              <span>来自 4 笔完成交易 · 未实现 +3.20</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              今日成本<span className="unit">COST</span>
            </div>
            <div className="kpi-value neg">{fmt.num(FEE_TOTAL)}</div>
            <div className="kpi-foot">
              <span>占毛利 {fmt.pct((FEE_TOTAL / GROSS_PROFIT) * 100, 1)}</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">
              账本差异<span className="unit">DIFF</span>
            </div>
            <div className="kpi-value">0.00</div>
            <div className="kpi-foot">
              <Chip variant="ok" dot>RECONCILED</Chip>
              <span>最近 16:41:24</span>
            </div>
          </div>
        </div>

        <div className="recon">
          <Icon name="check" size={14} />
          <span>
            账本与 Paper 撮合结果一致：可用余额、冻结余额、待结算余额与成交数量全部匹配，差异 <b>0.00 USDC</b>。
          </span>
        </div>

        <div className="grid g-main">
          <section className="panel">
            <header className="panel-head">
              <h2>
                流水 <span className="count">{rows.length} / {data.ledger.length}</span>
              </h2>
              <div className="panel-meta">
                <div className="seg" role="group" aria-label="账户筛选">
                  {ACCOUNT_FILTERS.map((f) => (
                    <button
                      key={f.key}
                      className={acct === f.key ? "is-active" : ""}
                      onClick={() => setAcct(f.key)}
                    >
                      {f.label}
                    </button>
                  ))}
                </div>
                <div className="seg" role="group" aria-label="类型筛选">
                  {TYPE_FILTERS.map((f) => (
                    <button
                      key={f.key}
                      className={type === f.key ? "is-active" : ""}
                      onClick={() => setType(f.key)}
                    >
                      {f.label}
                    </button>
                  ))}
                </div>
              </div>
            </header>
            <div className="panel-body panel-body--flush">
              <div className="tbl-wrap">
                <table className="tbl tbl-compact">
                  <thead>
                    <tr>
                      <th className="is-num">时间</th>
                      <th>编号</th>
                      <th>账户</th>
                      <th>类型</th>
                      <th>摘要</th>
                      <th className="is-num">金额</th>
                      <th className="is-num">余额</th>
                      <th>关联</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((l) => (
                      <tr key={l.id}>
                        <td className="is-num muted">{l.ts}</td>
                        <td className="cell-id">{l.id}</td>
                        <td>{l.account}</td>
                        <td>
                          <span className="lg-type">{l.type}</span>
                        </td>
                        <td className="cell-strong">{l.memo}</td>
                        <td
                          className={`is-num ${
                            l.amount > 0 ? "pos" : l.amount < 0 ? "neg" : "muted"
                          }`}
                        >
                          {l.amount === 0
                            ? "—"
                            : fmt.signed(l.amount, l.unit ? 4 : 2) + (l.unit ? " " + l.unit : "")}
                        </td>
                        <td className="is-num muted">{fmt.num(l.balance, l.unit ? 4 : 2)}</td>
                        <td className="cell-id">{l.ref}</td>
                      </tr>
                    ))}
                    {rows.length === 0 && (
                      <tr>
                        <td colSpan={8}>
                          <div className="empty">当前筛选条件下没有流水记录。</div>
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
            <div className="panel-note">
              金额为带符号变动；资产类流水以原始单位记账（如 ETH、WETH），不与稳定币混算。
            </div>
          </section>

          <div className="stack">
            <section className="panel">
              <header className="panel-head">
                <h2>成本构成</h2>
                <div className="panel-meta">
                  今日 · {fmt.num(FEE_TOTAL)} USDC
                </div>
              </header>
              <div className="panel-body">
                <div className="stack" style={{ gap: 11 }}>
                  {FEES.map((f) => (
                    <div className="meter-row" key={f.label}>
                      <span className="meter-label">{f.label}</span>
                      <span className="meter-val">{fmt.num(f.v)}</span>
                      <div className="bar">
                        <i style={{ width: `${(f.v / FEES[0].v) * 100}%` }} />
                      </div>
                    </div>
                  ))}
                </div>
                <div className="divider" />
                <dl className="kv">
                  <dt>成本合计</dt>
                  <dd>{fmt.num(FEE_TOTAL)}</dd>
                  <dt>已实现毛利</dt>
                  <dd>22.72</dd>
                  <dt>成本占比</dt>
                  <dd>{fmt.pct((FEE_TOTAL / GROSS_PROFIT) * 100, 1)}</dd>
                  <dt>失败 Gas（单向）</dt>
                  <dd className="neg">−1.24</dd>
                </dl>
              </div>
              <div className="panel-note">
                闪电贷本金与偿还流水不计入收益；失败回滚的 Gas 必须计入成本。
              </div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>策略账本</h2>
                <div className="panel-meta">独立核算 · 不跨策略转移损失</div>
              </header>
              <div className="panel-body panel-body--flush">
                <div className="tbl-wrap">
                  <table className="tbl tbl-compact">
                    <thead>
                      <tr>
                        <th>策略</th>
                        <th className="is-num">交易</th>
                        <th className="is-num">费用</th>
                        <th className="is-num">净利</th>
                      </tr>
                    </thead>
                    <tbody>
                      {STRATEGY_BOOK.map((s) => (
                        <tr key={s.label}>
                          <td className="cell-strong">{s.label}</td>
                          <td className="is-num muted">{s.trades}</td>
                          <td className="is-num neg">{s.fee}</td>
                          <td className="is-num pos">{s.net}</td>
                        </tr>
                      ))}
                      <tr>
                        <td className="muted">合计</td>
                        <td className="is-num muted">214</td>
                        <td className="is-num neg">−392.90</td>
                        <td className="is-num pos">+182.63</td>
                      </tr>
                    </tbody>
                  </table>
                </div>
              </div>
              <div className="panel-note">每个策略独立账本；多策略之间不能通过账本转移掩盖真实损失。</div>
            </section>
          </div>
        </div>

        <div className="grid g-even">
          <section className="panel">
            <header className="panel-head">
              <h2>
                账户体系 <span className="count">{data.ledgerAccounts.length}</span>
              </h2>
              <div className="panel-meta">TDD §9.1</div>
            </header>
            <div className="panel-body panel-body--flush">
              <div className="tbl-wrap">
                <table className="tbl tbl-compact">
                  <thead>
                    <tr>
                      <th>账户</th>
                      <th>类型</th>
                      <th className="is-num">余额</th>
                      <th>说明</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.ledgerAccounts.map((a) => (
                      <tr key={a.name}>
                        <td className="cell-strong">{a.name}</td>
                        <td>
                          <span className="lg-type">{a.type}</span>
                        </td>
                        <td
                          className={`is-num ${
                            a.balance > 0 ? "" : a.balance < 0 ? "neg" : "muted"
                          }`}
                        >
                          {a.balance < 0 ? fmt.signed(a.balance) : fmt.num(a.balance)}
                        </td>
                        <td className="muted" style={{ fontSize: 10 }}>
                          {a.note}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
            <div className="panel-note">
              费用类账户记录累计支出，用于成本归因；资产账户余额已扣除对应费用，不重复计入可用资金。
            </div>
          </section>

          <div className="stack">
            <section className="panel">
              <header className="panel-head">
                <h2>
                  闪电贷流水 <span className="count">{data.flashLoans.length}</span>
                </h2>
                <div className="panel-meta">借入 → 路由 → 偿还</div>
              </header>
              <div className="panel-body panel-body--flush">
                <div className="tbl-wrap">
                  <table className="tbl tbl-compact">
                    <thead>
                      <tr>
                        <th>编号</th>
                        <th>Provider</th>
                        <th className="is-num">本金</th>
                        <th className="is-num">费用</th>
                        <th>状态</th>
                      </tr>
                    </thead>
                    <tbody>
                      {data.flashLoans.map((f) => (
                        <tr key={f.id}>
                          <td className="cell-id">{f.id}</td>
                          <td>
                            {f.provider}
                            <span className="cell-sub">
                              {f.token} · 块 {fmt.compact(f.block)}
                            </span>
                          </td>
                          <td className="is-num">
                            {fmt.num(f.amount, f.token === "WETH" ? 4 : 2)}
                          </td>
                          <td className="is-num muted">
                            {fmt.num(f.fee)} <span style={{ fontSize: 9 }}>{f.feeBps} bps</span>
                          </td>
                          <td>
                            <Chip variant={f.state === "REPAID" ? "ok" : "bad"} dot>
                              {f.state === "REPAID" ? "已偿还" : "已回滚"}
                            </Chip>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
              <div className="panel-note">
                闪电贷本金不计入收益；费用按回调实际值记账，不硬编码为固定比例。回滚交易仍计入 Gas。
              </div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>记账规则</h2>
                <div className="panel-meta">TDD §9.2</div>
              </header>
              <div className="panel-body">
                <div className="check-list">
                  {RULES.map((r) => (
                    <div className="check-item" key={r}>
                      <Icon name="check" size={11} />
                      <span>{r}</span>
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
