import { useState } from "react";
import { data, actions } from "../data";
import { useScanState } from "../data/store";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { scanDexEvents } from "../lib/scanEvents";
import { ScanStrip } from "../components/ScanStrip";
import { OppTable } from "../components/OppTable";
import { toast } from "../ui/Toast";

export function ScannerDex() {
  const { scanDex } = useScanState();
  const [busy, setBusy] = useState(false);
  const rs = data.routeSpace;
  const rows = data.opps.filter((o) => o.strategy !== "cex_dex");
  const actionable = rows.filter((o) => o.status === "ACTIONABLE").length;
  const pools = data.pools.filter((p) => p.state === "ENABLED").slice(0, 6);
  const block = data.chains.find((c) => c.block > 0)?.block ?? 0;
  const subjects = pools.length
    ? pools.slice(0, 4).map((p) => `${p.chain} · ${p.protocol} · ${p.pair}`)
    : ["Arbitrum · Uniswap V3 · WETH/USDC"];

  const toggle = async () => {
    if (busy) return;
    const next = !scanDex;
    setBusy(true);
    try {
      await actions.setScan(next);
      toast(next ? "DEX 原子扫描已启动" : "DEX 原子扫描已停止", {
        sub: next ? "重新校验链头与池状态后再生成机会" : "已持久化的机会与账本记录不受影响",
        variant: next ? undefined : "warn",
      });
    } catch (e) {
      toast("后端未响应", { sub: e instanceof Error ? e.message : String(e), variant: "warn" });
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow"><span className="idx">02</span> MONITOR · DEX ATOMIC SCANNER</div>
          <h1>DEX 原子扫描</h1>
          <p className="page-sub">同链、同一笔 EVM 交易内完成的三角与跨池闭环。任一跳失败整笔回滚，但仍产生 Gas 成本。</p>
        </div>
        <div className="page-actions">
          <div className="scan-state">
            <span className={`scan-pulse${scanDex ? "" : " scan-pulse--off"}`} />
            <b style={{ color: scanDex ? "var(--pos)" : "var(--ink-4)" }}>{scanDex ? "SCANNING" : "STOPPED"}</b>
          </div>
          <button
            className={`btn ${scanDex ? "btn--danger" : "btn--primary"}`}
            onClick={toggle}
            disabled={busy}
          >
            <Icon name={scanDex ? "pause" : "play"} />
            {scanDex ? "停止扫描" : "启动扫描"}
          </button>
        </div>
      </div>

      <ScanStrip
        running={scanDex}
        subjects={subjects}
        events={scanDexEvents}
        quotes={data.scanner.quotes}
        quotesLabel="quotes"
        pools={data.scanner.pools}
        block={block}
        latency={data.scanner.lastScanMs ?? 0}
      />

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">扫描链<span className="unit">CHAINS</span></div>
            <div className="kpi-value">{data.scanner.chains || 1}<small>/ {data.chains.length || 4}</small></div>
            <div className="kpi-foot"><span>仅 PAPER_READY 及以上的链产出可执行机会</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">参与池<span className="unit">POOLS</span></div>
            <div className="kpi-value">{data.scanner.pools}</div>
            <div className="kpi-foot"><span>7 协议 · 起始资产 {rs.tokens}</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">本轮候选路线<span className="unit">ROUTES</span></div>
            <div className="kpi-value">{fmt.num(rs.candidates, 0)}</div>
            <div className="kpi-foot"><span>剪枝后 {fmt.num(rs.pruned, 0)} · 模拟 {rs.simulated}</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">可执行<span className="unit">ACTIONABLE</span></div>
            <div className="kpi-value">{actionable}</div>
            <div className="kpi-foot"><span>共 {rows.length} 条 DEX 机会 · {actionable} 条通过风控</span></div>
          </div>
        </div>

        <div className="grid g-main">
          <OppTable
            rows={rows}
            variant="dex"
            showStrategyFilter
            note="全部跳数在同一笔交易内完成；回滚不等于零成本，失败交易的 Gas 计入策略成本。跨链路线不属于原子套利。"
            emptyText="当前筛选条件下没有 DEX 原子机会。"
            onOpen={(id) => (location.hash = `#/opportunity/${id}`)}
          />

          <div className="stack">
            <section className="panel">
              <header className="panel-head">
                <h2>本轮扫描规模</h2>
                <div className="panel-meta">{data.scanner.uptime}</div>
              </header>
              <div className="panel-body">
                <div className="funnel">
                  <div className="funnel-row"><span>本轮候选</span><div className="funnel-bar funnel-bar--dim"><i style={{ width: "100%" }} /></div><b>{fmt.num(rows.length, 0)}</b></div>
                  <div className="funnel-row"><span>通过模拟</span><div className="funnel-bar funnel-bar--dim"><i style={{ width: "62%" }} /></div><b>{fmt.num(rows.filter((o) => o.status !== "EXPIRED").length, 0)}</b></div>
                  <div className="funnel-row"><span>已拒绝</span><div className="funnel-bar funnel-bar--bad"><i style={{ width: "34%" }} /></div><b>{rows.filter((o) => o.status === "REJECTED").length}</b></div>
                  <div className="funnel-row"><span>风控通过</span><div className="funnel-bar"><i style={{ width: `${Math.min(100, (actionable / Math.max(1, rows.length)) * 100)}%` }} /></div><b>{actionable}</b></div>
                </div>
              </div>
              <div className="panel-body panel-body--tight" style={{ borderTop: "1px solid var(--line-1)" }}>
                <div className="row between">
                  <span className="hint">起始资产 {rs.tokens} · 最大 {rs.hops} 跳</span>
                  <span className="hint">{fmt.num(data.scanner.quotes, 0)} quotes/s</span>
                </div>
              </div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>池状态</h2>
                <div className="panel-meta">按 TVL 排序 · 前 6</div>
              </header>
              <div className="panel-body panel-body--flush">
                <div className="ev-list">
                  {pools.map((p) => (
                    <div className="ev" key={p.addr}>
                      <div className="ev-top">
                        <span className="ev-scope mono" style={{ fontSize: 10.5 }}>{p.pair}</span>
                        <span className="chip chip--idle">{p.protocol}</span>
                        <span className="ev-time">{p.age}</span>
                      </div>
                      <div className="ev-msg">{p.chain} · TVL {fmt.compact(p.tvl)} · {p.fee}</div>
                    </div>
                  ))}
                </div>
              </div>
              <div className="panel-note">池状态以报价区块为准；主备 RPC 不一致或代码哈希变化时暂停该池。</div>
            </section>
          </div>
        </div>
      </div>
    </>
  );
}
