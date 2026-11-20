import { useEffect, useState } from "react";

interface SummaryItem {
  metric: string;
  unit: string;
  count: number;
  p50?: string;
  p95?: string;
  p99?: string;
  min?: string;
  max?: string;
}

const show = (v?: string) => (v === undefined || v === null || v === "" ? "—" : String(v));

export function Metrics() {
  const [items, setItems] = useState<SummaryItem[]>([]);
  const [report, setReport] = useState<Record<string, unknown> | null>(null);
  const [attr, setAttr] = useState<{ total: number; items: { class: string; count: number }[] } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    const load = async () => {
      try {
        const [s, r, a] = await Promise.all([
          fetch("/api/metrics/summary?window_sec=300").then((x) => x.json()),
          fetch("/api/metrics/report").then((x) => x.json()),
          fetch("/api/metrics/attribution").then((x) => x.json()),
        ]);
        setItems((s.items ?? []) as SummaryItem[]);
        setReport(r as Record<string, unknown>);
        setAttr(a as { total: number; items: { class: string; count: number }[] });
        setErr(null);
      } catch (e) {
        setErr(e instanceof Error ? e.message : "接口不可用");
      }
    };
    void load();
    const t = window.setInterval(load, 5000);
    return () => window.clearInterval(t);
  }, []);

  return (
    <div className="stack">
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">RESEARCH · V22-101</div>
          <h1>指标体系</h1>
          <p className="page-sub">实时指标、滚动报告与 12 类失败归因；数据全部来自 /api/metrics/*。</p>
        </div>
      </div>

      {err && (
        <section className="panel">
          <div className="panel-body muted">接口不可用：{err}</div>
        </section>
      )}

      <section className="panel">
        <div className="panel-head">
          <h2>窗口指标（5 分钟）</h2>
          <div className="panel-meta">{items.length} 项 · 每 5s 刷新</div>
        </div>
        <div className="panel-body">
          <table className="tbl">
            <thead>
              <tr>
                <th>指标</th>
                <th className="is-num">样本</th>
                <th className="is-num">P50</th>
                <th className="is-num">P95</th>
                <th className="is-num">P99</th>
                <th className="is-num">最小</th>
                <th className="is-num">最大</th>
                <th>单位</th>
              </tr>
            </thead>
            <tbody>
              {items.map((s) => (
                <tr key={s.metric}>
                  <td className="mono">{s.metric}</td>
                  <td className="is-num num">{s.count}</td>
                  <td className="is-num num">{show(s.p50)}</td>
                  <td className="is-num num">{show(s.p95)}</td>
                  <td className="is-num num">{show(s.p99)}</td>
                  <td className="is-num num">{show(s.min)}</td>
                  <td className="is-num num">{show(s.max)}</td>
                  <td className="muted">{s.unit}</td>
                </tr>
              ))}
              {items.length === 0 && (
                <tr>
                  <td colSpan={8} className="muted">
                    暂无指标（扫描器未运行或尚未产生数据）
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel">
        <div className="panel-head">
          <h2>滚动报告</h2>
          <div className="panel-meta">{String(report?.verdict ?? "—")}</div>
        </div>
        <div className="panel-body">
          <table className="tbl">
            <tbody>
              {[
                ["样本量", report?.sample_size],
                ["净利（USDC）", report?.net_pnl],
                ["失败成本（USDC）", report?.failure_cost],
                ["回撤", report?.drawdown],
                ["权益", report?.equity],
              ].map(([k, v]) => (
                <tr key={String(k)}>
                  <td className="muted">{String(k)}</td>
                  <td className="is-num num">{v === undefined || v === null ? "—" : String(v)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel">
        <div className="panel-head">
          <h2>失败归因（12 类）</h2>
          <div className="panel-meta">{attr?.total ?? 0} 条</div>
        </div>
        <div className="panel-body">
          <table className="tbl">
            <thead>
              <tr>
                <th>类别</th>
                <th className="is-num">次数</th>
              </tr>
            </thead>
            <tbody>
              {(attr?.items ?? []).map((i) => (
                <tr key={i.class}>
                  <td className="mono">{i.class}</td>
                  <td className="is-num num">{i.count}</td>
                </tr>
              ))}
              {(attr?.items ?? []).length === 0 && (
                <tr>
                  <td colSpan={2} className="muted">
                    暂无失败成本事件
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
