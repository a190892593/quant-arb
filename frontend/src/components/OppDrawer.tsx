import { useEffect, useState } from "react";
import { data, actions } from "../data";
import { fmt, signClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { capacityFor, failureModels, gatesFor, legsFor, scoreFor } from "../lib/opportunity";
import { LineChart } from "../charts/LineChart";
import { Waterfall } from "../charts/Waterfall";
import { QualityChip, StatusChip } from "../ui/Chip";
import { Drawer } from "../ui/Drawer";
import { toast } from "../ui/Toast";

type Tab = "eval" | "route" | "cap" | "risk" | "plan";

const TABS: { id: Tab; label: string }[] = [
  { id: "eval", label: "推演" },
  { id: "route", label: "路线" },
  { id: "cap", label: "容量" },
  { id: "risk", label: "风控" },
  { id: "plan", label: "计划" },
];

export function OppDrawer({ id, onClose }: { id: string; onClose: () => void }) {
  const o = data.opps.find((x) => x.id === id);
  const [tab, setTab] = useState<Tab>("eval");
  const [remain, setRemain] = useState(() => 12 + (parseInt(id.slice(-2), 36) % 17));

  useEffect(() => {
    setTab("eval");
  }, [id]);

  useEffect(() => {
    if (!o || o.status !== "ACTIONABLE") return;
    const t = window.setInterval(() => setRemain((r) => (r > 0 ? r - 1 : 0)), 1000);
    return () => clearInterval(t);
  }, [id, o]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  if (!o) return null;

  const legs = legsFor(o);
  const gates = gatesFor(o, data.chains);
  const fails = gates.filter((g) => !g.ok);
  const scores = scoreFor(o);
  const cap = capacityFor(o);
  const peak = cap.reduce((a, b) => (b.net > a.net ? b : a), cap[0]);
  const failures = failureModels(o);
  const worst = failures.reduce((s, f) => s + f.loss, 0);
  const expected = failures.reduce((s, f) => s + f.loss * (f.prob / 100), 0);

  const costKeys: [keyof typeof o.costs, string][] = [
    ["dexFee", "DEX 池费"],
    ["cexFee", "CEX 手续费"],
    ["gas", "Gas"],
    ["priority", "Priority / Builder"],
    ["flash", "闪电贷费用"],
    ["slippage", "模拟滑点"],
    ["hedge", "对冲风险准备金"],
    ["rebalance", "库存再平衡"],
  ];
  const costs = costKeys.filter(([k]) => o.costs[k] > 0);

  const wrap = (children: React.ReactNode) => <div className="drawer-body">{children}</div>;

  const body = () => {
    if (tab === "eval") {
      return wrap(
        <>
          <div className="dw-ribbon">
            {[
              { label: "理论价差", m: "M0", v: o.theo, note: "未扣成本", bps: o.theoBps },
              { label: "可成交毛利", m: "M1", v: o.gross, note: "按链上池状态 / 盘口深度", bps: (o.gross / o.notional) * 10000 },
              { label: "净利润", m: "—", v: o.net, note: `已扣 ${fmt.num(o.costTotal)} 成本（非模拟等级）`, bps: o.netBps },
              { label: "压力净利", m: "M3", v: o.stressNet, note: "50% 折扣 + 延迟 + 回滚", bps: (o.stressNet / o.notional) * 10000 },
            ].map((c) => (
              <div className="dw-cell" key={c.label}>
                <span>
                  {c.label} <em className="tag-mini">{c.m}</em>
                </span>
                <b className={signClass(c.v)}>{fmt.signed(c.v)}</b>
                <small>{fmt.num(c.bps, 1)} bps · {c.note}</small>
              </div>
            ))}
          </div>

          {(() => {
            const mp = o.rejections?.find((r) => r.key === "min_profit");
            const gapUsd = mp ? mp.limit - mp.actual : null;
            const gapBps =
              gapUsd !== null && o.notional > 0 ? (gapUsd / o.notional) * 10000 : null;
            return (
              <div className="hint mt-8">
                毛边 {fmt.num((o.gross / o.notional) * 10000, 2)} bps · 成本{" "}
                {fmt.num((o.costTotal / o.notional) * 10000, 2)} bps · 净利{" "}
                {fmt.num(o.netBps, 2)} bps
                {gapBps !== null &&
                  ` · 距执行门槛 ${fmt.num(gapBps, 2)} bps（${fmt.num(gapUsd ?? 0, 2)} USDC）`}
                {mp?.severity === "WARNING" && " · 已达观察层"}
              </div>
            );
          })()}

          <section className="dw-sec">
            <div className="dw-sec-head">
              成本桥 <span>合计 {fmt.num(o.costTotal)} · 占毛利 {fmt.pct((o.costTotal / o.gross) * 100, 1)}</span>
            </div>
            <Waterfall
              steps={[
                { label: "理论价差", value: o.theo, kind: "start" },
                { label: "可成交折扣", value: o.gross - o.theo, kind: "delta" },
                { label: "成本合计", value: -o.costTotal, kind: "delta" },
                { label: "净利润", value: o.net, kind: "total" },
                { label: "压力折扣", value: o.stressNet - o.net, kind: "delta" },
                { label: "压力净利", value: o.stressNet, kind: "total" },
              ]}
            />
          </section>

          <section className="dw-sec">
            <div className="dw-sec-head">成本明细</div>
            <table className="tbl tbl-compact">
              <thead>
                <tr><th>成本项</th><th className="is-num">金额</th><th className="is-num">占毛利</th></tr>
              </thead>
              <tbody>
                {costs.map(([k, label]) => (
                  <tr key={k}>
                    <td>{label}</td>
                    <td className="is-num neg">{fmt.signed(-o.costs[k])}</td>
                    <td className="is-num muted">{fmt.pct((o.costs[k] / o.gross) * 100, 1)}</td>
                  </tr>
                ))}
                <tr>
                  <td className="cell-strong">成本合计</td>
                  <td className="is-num neg cell-strong">{fmt.signed(-o.costTotal)}</td>
                  <td className="is-num muted">{fmt.pct((o.costTotal / o.gross) * 100, 1)}</td>
                </tr>
              </tbody>
            </table>
          </section>

          <section className="dw-sec">
            <div className="dw-sec-head">
              情景推演 <span>M0 → M3</span>
            </div>
            <table className="tbl tbl-compact scenario-tbl">
              <thead>
                <tr><th>模型</th><th>口径</th><th className="is-num">净利</th><th className="is-num">bps</th></tr>
              </thead>
              <tbody>
                {[
                  { m: "M0", name: "理论价差", note: "报价瞬间价差，未扣任何成本", v: o.theo, bps: o.theoBps },
                  { m: "M1", name: "链上实时状态", note: "V2 getReserves / V3 slot0+liquidity（只读）", v: o.gross, bps: (o.gross / o.notional) * 10000 },
                  {
                    m: "M2",
                    name: o.m2 ? (o.m2.passed ? "REVM 分叉复核（通过）" : "REVM 分叉复核（未通过）") : "REVM 分叉复核（待运行）",
                    note: o.m2
                      ? `block ${o.m2.block} · M1 ${fmt.num(o.m2.m1Out)} → M2 ${fmt.num(o.m2.m2Out)} · ${o.m2.diffBps.toFixed(2)} bps · gas ${o.m2.gasUsed}`
                      : "扫描按间隔调用 QuoterV2 复核，通过后标记 VERIFIED",
                    v: null,
                    bps: null,
                  },
                  { m: "M3", name: "压力模型", note: "50% 折扣 + 延迟 250 ms + 回滚 8%", v: o.stressNet, bps: (o.stressNet / o.notional) * 10000 },
                ].map((s) => (
                  <tr key={s.m}>
                    <td><div className="m-label"><span className="m-grade">{s.m}</span>{s.name}</div></td>
                    <td className="muted">{s.note}</td>
                    <td className={`is-num ${s.v === null ? "muted" : signClass(s.v)}`}>{s.v === null ? "—" : fmt.signed(s.v)}</td>
                    <td className="is-num muted">{s.bps === null ? "—" : fmt.num(s.bps, 1)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="hint mt-8">M2 由 REVM 在报价区块的链上状态上重放 QuoterV2；通过后机会标记为 VERIFIED（A），未通过则保留 M1 并给出 m2_mismatch 拒绝。</div>
          </section>
        </>,
      );
    }

    if (tab === "route") {
      return wrap(
        <>
          <section className="dw-sec">
            <div className="dw-sec-head">
              执行路线 <span>{legs.length} 跳 · 起始 {legs[0]?.from ?? "—"}</span>
            </div>
            <div className="legs">
              {legs.map((l) => (
                <div className="leg" key={l.n}>
                  <span className="leg-n">0{l.n}</span>
                  <div>
                    <div className="leg-flow">
                      <span className="tkn">{l.from}</span>
                      <span className="leg-link"><em>{l.pool} · {l.fee}</em></span>
                      <span className="tkn">{l.to}</span>
                    </div>
                    <div className="leg-meta">
                      {l.inQty} → <b>{l.outQty}</b> · 最低输出 {l.minOut} · 冲击 {l.impact}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </section>
          <div className="note">
            {o.strategy === "cex_dex"
              ? "CEX 腿按盘口逐档消耗，DEX 腿按区块状态模拟；两腿无共同原子提交机制，任一腿失败都需补偿流程。"
              : "全部跳数在同一笔 EVM 交易内完成；任一跳最低输出不满足则整笔回滚，但仍产生 Gas 成本。"}
          </div>
        </>,
      );
    }

    if (tab === "cap") {
      return wrap(
        <>
          <section className="dw-sec">
            <div className="dw-sec-head">
              净利润 vs 目标名义 <span>峰值 @ {fmt.num(peak.notional, 0)} · {fmt.signed(peak.net)}</span>
            </div>
            <LineChart
              values={cap.map((c) => c.net)}
              height={150}
              baseline={0}
              fillToBaseline
              yFormat={(v) => fmt.num(v, 1)}
              hoverFormat={(v) => `${fmt.signed(v, 2)} USDC`}
              hoverLabel={(i) => `${fmt.num(cap[i].notional, 0)} USDC 名义`}
              xLabels={[
                { text: "250", at: 1 },
                { text: "1,000", at: 40 },
                { text: "2,500", at: 60 },
                { text: "10,000", at: 99 },
              ]}
            />
          </section>
          <section className="dw-sec">
            <div className="dw-sec-head">
              失效模型 <span>最坏 {fmt.signed(worst)} · 期望 {fmt.signed(expected)} / 笔</span>
            </div>
            <table className="tbl tbl-compact">
              <thead>
                <tr><th>场景</th><th className="is-num">概率</th><th className="is-num">单次损失</th><th>处置</th></tr>
              </thead>
              <tbody>
                {failures.map((f) => (
                  <tr key={f.name}>
                    <td className="cell-strong">{f.name}</td>
                    <td className="is-num muted">{fmt.pct(f.prob, 1)}</td>
                    <td className="is-num neg">{fmt.signed(f.loss)}</td>
                    <td className="muted" style={{ whiteSpace: "normal", fontSize: 10 }}>{f.note}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
          <div className="hint">容量按池深度与固定 Gas 推算：名义越大价格冲击越显著，低于固定成本规模时不产生净利润。</div>
        </>,
      );
    }

    if (tab === "risk") {
      return wrap(
        <>
          <section className="dw-sec">
            <div className="dw-sec-head">
              风控闸门 <span>{gates.length - fails.length}/{gates.length} 通过</span>
            </div>
            {fails.length > 0 && (
              <div className="note mb-12" style={{ borderColor: "oklch(68% 0.165 25 / .35)" }}>
                <b>被拦截 · {fails.length} 条</b>
                {(o.reasons && o.reasons.length ? o.reasons : fails.map((f) => `${f.name} ${f.val}`)).map((r, i) => (
                  <div key={r} style={{ marginTop: 4 }}>{i + 1}. {r}</div>
                ))}
              </div>
            )}
            <div className="gates">
              {gates.map((g) => (
                <div className="gate" key={g.name}>
                  <span className={`gate-ico${g.ok ? "" : " is-bad"}`}>
                    <Icon name={g.ok ? "check" : "x"} size={11} />
                  </span>
                  <span className="gate-name">{g.name}<small>{g.detail}</small></span>
                  <span className={`gate-val${g.ok ? "" : " is-bad"}`}>{g.val}</span>
                </div>
              ))}
            </div>
          </section>

          <section className="dw-sec">
            <div className="dw-sec-head">评分构成 <span>{o.riskScore} / 100</span></div>
            <div className="stack" style={{ gap: 9 }}>
              {scores.map((s) => (
                <div className="meter-row" key={s.name}>
                  <span className="meter-label">{s.name}</span>
                  <span className="meter-val">{s.part} / {s.weight}</span>
                  <div className={`bar${s.part / s.weight >= 0.75 ? " bar--ok" : s.part / s.weight >= 0.5 ? "" : " bar--warn"}`}>
                    <i style={{ width: `${(s.part / s.weight) * 100}%` }} />
                  </div>
                </div>
              ))}
            </div>
          </section>

          <section className="dw-sec">
            <div className="dw-sec-head">数据溯源</div>
            <dl className="kv">
              <dt>链 / 区块</dt><dd>{data.CHAIN[o.chain].label} · {fmt.num(o.block, 0)}</dd>
              <dt>区块差 / 报价年龄</dt><dd>{o.blockDelta} 块 · {fmt.age(o.quoteAgeMs)}</dd>
              <dt>数据质量 / 模拟等级</dt><dd>Q{o.quality} · {o.model}</dd>
              <dt>主备 RPC</dt><dd>{o.facts.rpcOk ? "一致" : "不一致"}</dd>
              <dt>报价哈希</dt><dd>0x8f2c…a71b</dd>
            </dl>
          </section>
        </>,
      );
    }

    return wrap(
      <>
        <section className="dw-sec">
          <div className="dw-sec-head">
            交易计划 <span>合约 §5 · {o.status === "ACTIONABLE" ? "待签名" : "未生成"}</span>
          </div>
          <dl className="kv">
            <dt>plan ID</dt><dd>TP-{o.id.slice(4)}</dd>
            <dt>chainId / executor</dt><dd>{data.CHAIN[o.chain].id} · 0x9A3f…21c4</dd>
            <dt>strategyId</dt><dd>{o.strategy.toUpperCase()}</dd>
            <dt>nonce / deadline</dt><dd>48213 · {o.status === "ACTIONABLE" ? "16:49:32" : "已过期"}</dd>
            <dt>startToken / amountIn</dt><dd>{legs[0]?.from ?? "—"} · {legs[0]?.inQty ?? "—"}</dd>
            <dt>minProfit</dt><dd>{fmt.num(o.minProfit)} USDC</dd>
            <dt>funding</dt><dd>{o.strategy === "cex_dex" ? "预置库存（两边分离）" : "Aave V3 · 单资产"}</dd>
            <dt>maxGasPrice / maxBlock</dt><dd>0.05 gwei · {fmt.num(o.block + 3, 0)}</dd>
          </dl>
        </section>

        <section className="dw-sec">
          <div className="dw-sec-head">每腿最低输出</div>
          <div className="leg-constraints">
            {legs.map((l) => (
              <div className="leg-constraint" key={l.n}>
                <span className="mono">{l.from}→{l.to}</span>
                <span className="mono muted">min {l.minOut}</span>
              </div>
            ))}
          </div>
        </section>

        <section className="dw-sec">
          <div className="dw-sec-head">签名</div>
          <div className="sig-block">
            <div><span>routeHash</span><b>0x8f2c…a71b</b></div>
            <div><span>signer</span><b>0x51c2…88ae</b></div>
            <div><span>signature</span><b>0x7d1a…3f90</b></div>
          </div>
        </section>

        <div className="hint">EIP-712 域绑定 Chain ID 与 Executor；计划有效期 ≤ 30 s，过期或 Nonce 已用则拒绝执行。</div>
      </>,
    );
  };

  return (
    <Drawer
      ariaLabel="机会详情"
      storageKey="opportunity"
      onClose={onClose}
      head={
        <>
          <div className="row" style={{ gap: 8 }}>
            <span className="drawer-id">{o.id}</span>
            <StatusChip status={o.status} />
            <QualityChip quality={o.quality} />
            <div className={`countdown${o.status === "ACTIONABLE" && remain <= 8 ? " is-low" : ""}`}>
              <span>计划有效期</span>
              <b>{o.status === "ACTIONABLE" ? (remain > 0 ? `${remain} s` : "已过期") : "已失效"}</b>
            </div>
          </div>
          <div className="drawer-sub">
            {data.STRATEGY[o.strategy].label} · {data.CHAIN[o.chain].label}
            {o.cex ? ` · ${o.cex}` : ""}
            <span className="sep" />
            {o.symbol ?? o.pair}
            <span className="sep" />
            {o.model} · 风险 {o.riskScore}
          </div>
        </>
      }
      footer={
        <>
          <span className="hint">{o.venue} · 名义 {fmt.num(o.notional, 0)} USDC</span>
          <button className="btn" onClick={() => toast("已加入观察列表", { sub: `${o.id} 将在数据恢复完整后重新评估`, variant: "warn" })}>
            <Icon name="target" size={13} /> 加入观察
          </button>
          <button
            className="btn btn--primary"
            disabled={o.status !== "ACTIONABLE"}
            onClick={async () => {
              try {
                await actions.execute(o.rawId ?? o.id);
                toast(`Paper 执行已生成 TP-${o.id.slice(4)}`, { sub: "跳转到执行明细查看双腿与账本流水" });
                setTimeout(() => (location.hash = "#/executions"), 420);
              } catch (e) {
                toast("执行被拒绝", {
                  sub: e instanceof Error ? e.message : String(e),
                  variant: "warn",
                });
              }
            }}
          >
            <Icon name="sim" size={13} /> 生成模拟执行
          </button>
        </>
      }
    >

      <div className="drawer-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            className={`drawer-tab${tab === t.id ? " is-active" : ""}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      {body()}

    </Drawer>
  );
}
