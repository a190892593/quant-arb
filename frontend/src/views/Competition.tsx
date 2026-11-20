import { useEffect, useState } from "react";

interface Overview {
  level: string;
  reason: string;
  tx_count_30d: number;
  active_addresses_30d: number;
  collector: string;
  note: string;
}

interface MetricsResp {
  total_transactions: number;
  high_confidence: number;
  distinct_blocks: number;
  level: string;
  reason: string;
}

interface Tx {
  tx_hash: string;
  block: number;
  tx_index: number;
  confidence: string;
  log_count: number;
  pools: string[];
  sender?: string;
}

interface Competitor {
  address: string;
  tx_count: number;
  last_block: number;
  pools: string[];
}

export function Competition() {
  const [overview, setOverview] = useState<Overview | null>(null);
  const [metrics, setMetrics] = useState<MetricsResp | null>(null);
  const [txs, setTxs] = useState<Tx[]>([]);
  const [competitors, setCompetitors] = useState<Competitor[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [decision, setDecision] = useState<Record<string, unknown> | null>(null);
  const [bid, setBid] = useState({
    p_success: "0.6",
    p_fail: "0.4",
    success_net: "20",
    failure_cost: "3",
    extra_bid_cost: "1",
    min_expected: "5",
    samples: "100",
    tier: "MEDIUM",
  });

  const load = async () => {
    try {
      const [o, m, t, c] = await Promise.all([
        fetch("/api/competition/overview").then((x) => x.json()),
        fetch("/api/competition/metrics").then((x) => x.json()),
        fetch("/api/competition/transactions?blocks=50").then((x) => x.json()),
        fetch("/api/competition/competitors?limit=200").then((x) => x.json()),
      ]);
      setOverview(o as Overview);
      setMetrics(m as MetricsResp);
      setTxs((t.items ?? []) as Tx[]);
      setCompetitors((c.addresses ?? []) as Competitor[]);
      setErr(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : "接口不可用");
    }
  };

  useEffect(() => {
    void load();
    const t = window.setInterval(load, 10000);
    return () => window.clearInterval(t);
  }, []);

  const collect = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await fetch("/api/competition/collect?blocks=500", { method: "POST" });
      await load();
    } finally {
      setBusy(false);
    }
  };

  const simulate = async () => {
    const payload = {
      p_success: Number(bid.p_success),
      p_fail: Number(bid.p_fail),
      success_net: bid.success_net,
      failure_cost: bid.failure_cost,
      extra_bid_cost: bid.extra_bid_cost,
      min_expected: bid.min_expected,
      samples: Number(bid.samples),
      p_from_history: true,
      tier: bid.tier,
    };
    const r = await fetch("/api/competition/bidding/simulate", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    setDecision(await r.json());
  };

  return (
    <div className="stack">
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">SEARCH · PH4 / PH6</div>
          <h1>竞争者分析</h1>
          <p className="page-sub">
            竞争者交易由 Swap 日志重建（同 tx ≥2 Swap、≥2 池）；地址 ≠ 机器人，聚类为概率识别。
          </p>
        </div>
        <div className="mt-8">
          <button className="btn" onClick={collect} disabled={busy}>
            {busy ? "采集中…" : "采集最近 500 块"}
          </button>
        </div>
      </div>

      {err && (
        <section className="panel">
          <div className="panel-body muted">接口不可用：{err}</div>
        </section>
      )}

      <section className="panel">
        <div className="panel-head">
          <h2>竞争强度</h2>
          <div className="panel-meta">{overview?.level ?? "—"}</div>
        </div>
        <div className="panel-body">
          <table className="tbl">
            <tbody>
              <tr><td className="muted">竞争者交易</td><td className="is-num num">{metrics?.total_transactions ?? 0}</td></tr>
              <tr><td className="muted">HIGH 置信</td><td className="is-num num">{metrics?.high_confidence ?? 0}</td></tr>
              <tr><td className="muted">涉及区块</td><td className="is-num num">{metrics?.distinct_blocks ?? 0}</td></tr>
              <tr><td className="muted">判定理由</td><td>{metrics?.reason ?? "—"}</td></tr>
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel">
        <div className="panel-head">
          <h2>竞争者交易</h2>
          <div className="panel-meta">{txs.length} 条</div>
        </div>
        <div className="panel-body">
          <table className="tbl">
            <thead>
              <tr>
                <th>交易</th>
                <th className="is-num">区块</th>
                <th className="is-num">Swap</th>
                <th>池</th>
                <th>置信</th>
                <th>发起方</th>
              </tr>
            </thead>
            <tbody>
              {txs.map((t) => (
                <tr key={t.tx_hash}>
                  <td className="mono">{t.tx_hash.slice(0, 12)}…</td>
                  <td className="is-num num">{t.block}</td>
                  <td className="is-num num">{t.log_count}</td>
                  <td className="mono">{(t.pools ?? []).length} 池</td>
                  <td>{t.confidence}</td>
                  <td className="mono">{(t.sender ?? "—").slice(0, 12)}…</td>
                </tr>
              ))}
              {txs.length === 0 && (
                <tr>
                  <td colSpan={6} className="muted">
                    暂无竞争者交易（点击「采集最近 500 块」）
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel">
        <div className="panel-head">
          <h2>地址画像</h2>
          <div className="panel-meta">{competitors.length} 个地址</div>
        </div>
        <div className="panel-body">
          <table className="tbl">
            <thead>
              <tr>
                <th>地址</th>
                <th className="is-num">交易数</th>
                <th className="is-num">最近区块</th>
                <th className="is-num">池数</th>
              </tr>
            </thead>
            <tbody>
              {competitors.map((c) => (
                <tr key={c.address}>
                  <td className="mono">{c.address.slice(0, 16)}…</td>
                  <td className="is-num num">{c.tx_count}</td>
                  <td className="is-num num">{c.last_block}</td>
                  <td className="is-num num">{(c.pools ?? []).length}</td>
                </tr>
              ))}
              {competitors.length === 0 && (
                <tr>
                  <td colSpan={4} className="muted">
                    暂无地址画像
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>

      <section className="panel">
        <div className="panel-head">
          <h2>竞价模拟（EV）</h2>
          <div className="panel-meta">{decision ? String(decision.decision) : "未运行"}</div>
        </div>
        <div className="panel-body">
          <table className="tbl">
            <tbody>
              <tr>
                <td className="muted">P(成功)</td>
                <td><input className="mono" value={bid.p_success} onChange={(e) => setBid({ ...bid, p_success: e.target.value })} /></td>
                <td className="muted">P(失败)</td>
                <td><input className="mono" value={bid.p_fail} onChange={(e) => setBid({ ...bid, p_fail: e.target.value })} /></td>
              </tr>
              <tr>
                <td className="muted">成功净利</td>
                <td><input className="mono" value={bid.success_net} onChange={(e) => setBid({ ...bid, success_net: e.target.value })} /></td>
                <td className="muted">失败成本</td>
                <td><input className="mono" value={bid.failure_cost} onChange={(e) => setBid({ ...bid, failure_cost: e.target.value })} /></td>
              </tr>
              <tr>
                <td className="muted">竞价成本</td>
                <td><input className="mono" value={bid.extra_bid_cost} onChange={(e) => setBid({ ...bid, extra_bid_cost: e.target.value })} /></td>
                <td className="muted">门槛</td>
                <td><input className="mono" value={bid.min_expected} onChange={(e) => setBid({ ...bid, min_expected: e.target.value })} /></td>
              </tr>
              <tr>
                <td className="muted">样本量</td>
                <td><input className="mono" value={bid.samples} onChange={(e) => setBid({ ...bid, samples: e.target.value })} /></td>
                <td className="muted">竞争档位</td>
                <td>
                  <select value={bid.tier} onChange={(e) => setBid({ ...bid, tier: e.target.value })}>
                    <option value="LOW">LOW</option>
                    <option value="MEDIUM">MEDIUM</option>
                    <option value="HIGH">HIGH</option>
                  </select>
                </td>
              </tr>
            </tbody>
          </table>
          <div className="mt-8">
            <button className="btn" onClick={simulate}>计算期望收益</button>
          </div>
          {decision && (
            <table className="tbl">
              <tbody>
                <tr><td className="muted">决策</td><td className="mono">{String(decision.decision)}</td></tr>
                <tr><td className="muted">期望净收益</td><td className="is-num num">{String(decision.expected_net)}</td></tr>
                <tr><td className="muted">档位</td><td className="mono">{String(decision.tier)}</td></tr>
                <tr><td className="muted">理由</td><td>{String(decision.reason)}</td></tr>
              </tbody>
            </table>
          )}
        </div>
      </section>
    </div>
  );
}
