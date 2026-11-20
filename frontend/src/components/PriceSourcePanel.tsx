import { useEffect, useState } from "react";
import type { PricesResponse } from "../types";
import { fetchPrices } from "../data/api";
import { fmt } from "../lib/format";

/** 标价源面板：链上为主、Chainlink 校验、CEX 兜底，并展示三源分歧。 */
export function PriceSourcePanel() {
  const [p, setP] = useState<PricesResponse | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () => {
      fetchPrices()
        .then((r) => {
          if (!alive) return;
          setP(r);
          setErr(null);
        })
        .catch((e) => alive && setErr(String(e)));
    };
    load();
    const t = setInterval(load, 5000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);

  const maxDiv = p?.max_divergence_bps ?? 0;
  const tone = maxDiv >= 50 ? "neg" : maxDiv >= 20 ? "warn" : "pos";
  const tokens = (p?.tokens ?? [])
    .filter((t) => t.usd > 0 && /^[A-Za-z0-9._-]{1,12}$/.test(t.symbol))
    .slice(0, 12);

  return (
    <section className="panel">
      <header className="panel-head">
        <h2>
          标价源 <span className="count">{p?.tokens.length ?? 0}</span>
        </h2>
        <div className="panel-meta">
          {err ? "后端不可达" : (p?.source ?? "LOADING")}
          <span className={`delta ${tone}`}>分歧 {fmt.num(maxDiv, 1)} bps</span>
        </div>
      </header>
      <div className="panel-body">
        <p className="hint" style={{ marginTop: 0 }}>
          计价货币（USDC / USDT / DAI）固定按 1 美元；其他 Token 取<strong>链上最深计价池的 spot 价</strong>为主源，
          Chainlink 聚合价做交叉校验，Binance 仅作第三票兜底。三源偏差 &gt; 50 bps 时，相关机会降级为「观察」。
        </p>
        <div className="tbl-wrap">
          <table className="tbl">
            <thead>
              <tr>
                <th>Token</th>
                <th className="is-num">USD 标价</th>
                <th className="is-num">三源分歧</th>
                <th>状态</th>
              </tr>
            </thead>
            <tbody>
              {tokens.map((t) => (
                <tr key={t.symbol}>
                  <td className="mono">{t.symbol}</td>
                  <td className="is-num">{t.usd >= 1 ? fmt.num(t.usd, 2) : fmt.num(t.usd, 6)}</td>
                  <td className={`is-num ${t.divergence_bps >= 50 ? "neg" : t.divergence_bps >= 20 ? "warn" : "muted"}`}>
                    {fmt.num(t.divergence_bps, 1)}
                  </td>
                  <td>
                    {t.divergence_bps >= 50 ? (
                      <span className="tag-mini is-bad">分歧超阈值</span>
                    ) : t.divergence_bps >= 20 ? (
                      <span className="tag-mini">轻微分歧</span>
                    ) : (
                      <span className="tag-mini">三源一致</span>
                    )}
                  </td>
                </tr>
              ))}
              {tokens.length === 0 && !err && (
                <tr>
                  <td colSpan={4} className="muted">
                    等待首次链上同步…
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        {p?.notes && <div className="panel-note">{p.notes}</div>}
      </div>
    </section>
  );
}
