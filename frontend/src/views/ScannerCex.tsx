import { useState } from "react";
import { data, actions } from "../data";
import { useScanState } from "../data/store";
import { fmt, signClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { scanCexEvents } from "../lib/scanEvents";
import { ScanStrip } from "../components/ScanStrip";
import { OppTable } from "../components/OppTable";
import { toast } from "../ui/Toast";

export function ScannerCex() {
  const { scanCex } = useScanState();
  const [busy, setBusy] = useState(false);
  const rows = data.opps.filter((o) => o.strategy === "cex_dex");
  const actionable = rows.filter((o) => o.status === "ACTIONABLE");
  const sumNet = actionable.reduce((s, o) => s + o.net, 0);
  const rejected = rows.filter((o) => o.status === "REJECTED").length;
  const watch = rows.filter((o) => o.status === "WATCH").length;
  const worstStress = actionable.length ? Math.min(...actionable.map((o) => o.stressNet)) : 0;
  const validBooks = data.instruments.filter((i) => i.integrity === "VALID").length;
  const block = data.chains.find((c) => c.block > 0)?.block ?? 0;
  const maxExposure = data.executions.length ? Math.max(0, ...data.executions.map((e) => e.exposure)) : 0;
  const exposureLimit = Math.max(1, (data.riskStatus?.equity ?? data.equity) * 0.005);
  const exposurePct = Math.min(100, (maxExposure / exposureLimit) * 100);
  const subjects = data.instruments.length
    ? data.instruments.slice(0, 3).map((i) => `${i.exchange} · ${i.symbol}`)
    : ["Binance · ETH/USDT"];

  const toggle = async () => {
    if (busy) return;
    const next = !scanCex;
    setBusy(true);
    try {
      await actions.setScan(next);
      toast(next ? "CEX–DEX 扫描已启动" : "CEX–DEX 扫描已停止", {
        sub: next ? "重新校验收盘口完整性与费率后再生效" : "已有订单与对冲流程不受影响",
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
          <div className="page-eyebrow"><span className="idx">03</span> MONITOR · CROSS-VENUE SCANNER</div>
          <h1>CEX–DEX 扫描</h1>
          <p className="page-sub">链外对冲不是原子套利：CEX 与 DEX 无共同提交机制，部分成交、回滚与状态未知都必须显式处置。</p>
        </div>
        <div className="page-actions">
          <div className="scan-state">
            <span className={`scan-pulse${scanCex ? "" : " scan-pulse--off"}`} />
            <b style={{ color: scanCex ? "var(--pos)" : "var(--ink-4)" }}>{scanCex ? "SCANNING" : "STOPPED"}</b>
          </div>
          <button
            className={`btn ${scanCex ? "btn--danger" : "btn--primary"}`}
            onClick={toggle}
            disabled={busy}
          >
            <Icon name={scanCex ? "pause" : "play"} />
            {scanCex ? "停止扫描" : "启动扫描"}
          </button>
        </div>
      </div>

      <ScanStrip
        running={scanCex}
        subjects={subjects}
        events={scanCexEvents}
        quotes={data.scanner.quotes}
        quotesLabel="quotes"
        pools={data.scanner.pools}
        poolLabel="交易对"
        block={block}
        latency={data.scanner.lastScanMs ?? 0}
      />

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">今日机会<span className="unit">CEX–DEX</span></div>
            <div className="kpi-value">{rows.length}</div>
            <div className="kpi-foot">
              <span className="delta pos">{actionable.length}</span><span>可执行 · </span>
              <span className="delta warn">{watch}</span><span>观察 · </span>
              <span className="delta neg">{rejected}</span><span>拒绝</span>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">可执行净利合计<span className="unit">USDC</span></div>
            <div className={`kpi-value ${signClass(sumNet)}`}>{fmt.signed(sumNet)}</div>
            <div className="kpi-foot"><span>已扣双边费用与准备金</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">最差压力净利<span className="unit">AMONG ACTIONABLE</span></div>
            <div className={`kpi-value ${actionable.length ? signClass(worstStress) : "muted"}`}>
              {actionable.length ? fmt.signed(worstStress) : "—"}
            </div>
            <div className="kpi-foot"><span>50% 折扣 + 延迟 + 回滚模型</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">未对冲敞口<span className="unit">LIVE</span></div>
            <div className="kpi-value">{fmt.num(maxExposure)}</div>
            <div className="kpi-foot">
              <span>上限 {fmt.num(exposureLimit)} · 占用 {fmt.num(exposurePct, 1)}%</span>
              <span className="kpi-spark" style={{ width: 64 }}>
                <span className={`bar${exposurePct >= 85 ? " bar--bad" : exposurePct >= 60 ? " bar--warn" : ""}`}>
                  <i style={{ width: `${Math.max(2, exposurePct)}%` }} />
                </span>
              </span>
            </div>
          </div>
        </div>

        <OppTable
          rows={rows}
          variant="cex"
          meta={
            <>
              盘口完整 {validBooks}/{data.instruments.length}
              <span className={`dot ${validBooks === data.instruments.length ? "dot--ok" : "dot--warn"}`} />
            </>
          }
          note="未对冲敞口 ≤ 25 USDC 且 ≤ 2 s；IOC 部分成交后必须按实际成交量重算另一腿；状态未知时先查询，禁止重发相同逻辑订单。"
          emptyText="当前筛选条件下没有 CEX–DEX 机会。"
          onOpen={(id) => (location.hash = `#/opportunity/${id}`)}
        />
      </div>
    </>
  );
}
