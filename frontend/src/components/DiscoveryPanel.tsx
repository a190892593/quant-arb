import { useEffect, useState } from "react";
import type { DiscoveryStatus } from "../types";
import { discoveryScan, discoveryStatus } from "../data/api";
import { toast } from "../ui/Toast";

export function DiscoveryPanel() {
  const [st, setSt] = useState<DiscoveryStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    try {
      setSt(await discoveryStatus());
    } catch {
      /* ignore */
    }
  };

  useEffect(() => {
    void load();
    const t = window.setInterval(() => {
      if (!document.hidden) void load();
    }, 10_000);
    return () => window.clearInterval(t);
  }, []);

  const scan = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const r = await discoveryScan();
      setSt(r);
      toast(`发现扫描完成：新增 ${r.new_pools} 池 / ${r.new_tokens} Token`, {
        variant: "info",
        sub: r.message,
      });
    } catch (e) {
      toast("发现扫描失败", { sub: String(e), variant: "bad" });
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel mt-12">
      <header className="panel-head">
        <h2>
          全链自动发现 <span className="count">DISCOVERY</span>
        </h2>
        <div className="panel-meta">
          {st ? (st.enabled ? (st.running ? "RUNNING" : "IDLE") : "DISABLED") : "LOADING"}
        </div>
      </header>
      <div className="panel-body">
        <div className="row">
          <button className="btn btn--sm" onClick={() => void scan()} disabled={busy}>
            {busy ? "扫描中…" : "立即扫描"}
          </button>
          <span className="hint">
            每 {10} 分钟自动扫描 Arbitrum 高 TVL 池；链上校验 token/fee/TVL 后自动加入白名单与池表。
          </span>
        </div>
        <dl className="kv mt-12">
          <dt>已发现池</dt>
          <dd>{st?.total_pools ?? 0}</dd>
          <dt>本轮候选</dt>
          <dd>{st?.scanned_logs ?? 0}</dd>
          <dt>本轮新增</dt>
          <dd>
            {st?.new_pools ?? 0} 池 / {st?.new_tokens ?? 0} Token
          </dd>
          <dt>数据源</dt>
          <dd>{st?.source || "—"}</dd>
          <dt>最新区块</dt>
          <dd>{st?.latest_block ?? "—"}</dd>
          <dt>最近运行</dt>
          <dd>{st?.last_run_at || "—"}</dd>
        </dl>
        <p className="hint mt-8">{st?.message || "尚未运行"}</p>
      </div>
    </section>
  );
}
