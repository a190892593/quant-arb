import { useEffect, useState } from "react";
import type {
  SearchConfigResponse,
  SearchParamsConfig,
  SearchTokenEntry,
} from "../types";
import {
  fetchSearchConfig,
  saveSearchConfig,
  updateSearchToken,
} from "../data/api";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

const FIELDS: { key: keyof SearchParamsConfig; label: string; step: number }[] = [
  { key: "min_pool_tvl_usd", label: "池 TVL 门槛 (USD)", step: 1000 },
  { key: "max_pool_share_pct", label: "单池深度占比 (%)", step: 1 },
  { key: "min_input_usd", label: "单笔下限 (USD)", step: 50 },
  { key: "max_input_usd", label: "单笔上限 (USD)", step: 500 },
  { key: "max_pool_state_age_ms", label: "池状态最大年龄 (ms)", step: 500 },
  { key: "max_hops", label: "最大跳数", step: 1 },
  { key: "min_hops", label: "最小跳数", step: 1 },
  { key: "max_pools", label: "参与池上限", step: 1 },
  { key: "max_evaluate", label: "单轮定价上限", step: 50 },
  { key: "max_evaluate_ms", label: "单轮定价预算 (ms)", step: 100 },
];

export function SearchConfigPanel({
  title = "搜索配置",
  keys,
  withTokens = true,
}: {
  title?: string;
  keys?: (keyof SearchParamsConfig)[];
  withTokens?: boolean;
} = {}) {
  const [cfg, setCfg] = useState<SearchConfigResponse | null>(null);
  const [draft, setDraft] = useState<SearchParamsConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [tok, setTok] = useState({ symbol: "", address: "", decimals: "18" });
  const [tokensOpen, setTokensOpen] = useState(false); // 白/黑名单默认折叠
  const [configOpen, setConfigOpen] = useState(false); // 搜索参数默认折叠

  const visible = keys ? FIELDS.filter((f) => keys.includes(f.key)) : FIELDS;
  const load = async () => {
    try {
      const c = await fetchSearchConfig();
      setCfg(c);
      setDraft(c.params);
    } catch {
      /* 后端未就绪时保持空态 */
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const patch = (key: keyof SearchParamsConfig, value: number) =>
    setDraft((d) => (d ? { ...d, [key]: value } : d));

  const save = async () => {
    if (!draft || busy) return;
    setBusy(true);
    try {
      await saveSearchConfig({ params: draft });
      await load();
      toast("搜索参数已保存并即时生效", { variant: "info" });
    } catch (e) {
      toast("保存搜索参数失败", { sub: String(e), variant: "bad" });
    } finally {
      setBusy(false);
    }
  };

  const tokenAction = async (
    action: "whitelist" | "blacklist" | "remove",
    t: SearchTokenEntry,
  ) => {
    try {
      await updateSearchToken({
        action,
        symbol: t.symbol,
        address: t.address,
        decimals: t.decimals,
      });
      await load();
      toast(
        action === "remove"
          ? `已移除 ${t.symbol}`
          : action === "blacklist"
            ? `${t.symbol} 已加入黑名单`
            : `${t.symbol} 已加入白名单`,
        { variant: "info", sub: "Token 列表变更需重启后端后生效" },
      );
    } catch (e) {
      toast("Token 操作失败", { sub: String(e), variant: "bad" });
    }
  };

  const addToken = async () => {
    const symbol = tok.symbol.trim().toUpperCase();
    const address = tok.address.trim();
    const decimals = Number(tok.decimals) || 18;
    if (!symbol || !address) {
      toast("请填写 Token 符号与地址", { variant: "warn" });
      return;
    }
    await tokenAction("whitelist", { symbol, address, decimals, status: "WHITELIST", source: "manual" });
    setTok({ symbol: "", address: "", decimals: "18" });
  };

  return (
    <section className="panel mt-12">
      <button
        className="panel-head panel-head--toggle"
        onClick={() => setConfigOpen((v) => !v)}
        aria-expanded={configOpen}
      >
        <h2>
          {title} <span className="count">可编辑</span>
        </h2>
        <div className="panel-meta">
          {cfg ? `白名单 ${cfg.whitelist} · 黑名单 ${cfg.blacklist}` : "LOADING"}
          <span className="ml-8">{configOpen ? "收起" : "展开"}</span>
          <b className={`chev${configOpen ? " is-open" : ""}`}>▾</b>
        </div>
      </button>
      <div className={`panel-body${configOpen ? "" : " is-hidden"}`}>
        {draft ? (
          <>
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "repeat(auto-fit, minmax(150px, 1fr))",
                gap: 10,
              }}
            >
              {visible.map((f) => (
                <label key={f.key} style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 11 }}>
                  <span>{f.label}</span>
                  <input
                    type="number"
                    step={f.step}
                    value={draft[f.key] as number}
                    onChange={(e) => patch(f.key, Number(e.target.value))}
                    style={{
                      height: 30,
                      padding: "0 8px",
                      border: "1px solid var(--line-2)",
                      borderRadius: 6,
                      background: "transparent",
                      color: "var(--ink-1)",
                      font: "inherit",
                    }}
                  />
                </label>
              ))}
            </div>
            <div className="row mt-8">
              <button className="btn btn--sm" onClick={() => void save()} disabled={busy}>
                {busy ? "保存中…" : "保存搜索参数"}
              </button>
              <span className="hint">
                参数保存后即时生效；Token 白/黑名单变更需重启后端重建 feed。
              </span>
            </div>

            {withTokens && (
            <>
            <button
              className="panel-head panel-head--toggle mt-12"
              onClick={() => setTokensOpen((v) => !v)}
              aria-expanded={tokensOpen}
            >
              <h3>
                Token 白/黑名单 <span className="count">{cfg?.tokens.length ?? 0}</span>
              </h3>
              <span className="panel-meta">
                {tokensOpen ? "收起" : "展开"}
                <b className={`chev${tokensOpen ? " is-open" : ""}`}>▾</b>
              </span>
            </button>
            <div className={`tbl-wrap${tokensOpen ? "" : " is-hidden"}`}>
              <table className="tbl">
                <thead>
                  <tr>
                    <th>Token</th>
                    <th>精度</th>
                    <th>状态</th>
                    <th>来源</th>
                    <th>地址</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {(cfg?.tokens ?? []).map((t) => (
                    <tr key={`${t.symbol}-${t.address}`}>
                      <td>{t.symbol}</td>
                      <td>{t.decimals}</td>
                      <td>
                        <Chip variant={t.status === "BLACKLIST" ? "bad" : "ok"}>
                          {t.status === "BLACKLIST" ? "黑名单" : "白名单"}
                        </Chip>
                      </td>
                      <td>{t.source}</td>
                      <td className="mono">
                        {t.address ? `${t.address.slice(0, 8)}…${t.address.slice(-6)}` : "—"}
                      </td>
                      <td>
                        <div className="row">
                          {t.status === "BLACKLIST" ? (
                            <button
                              className="btn btn--sm"
                              onClick={() => void tokenAction("whitelist", t)}
                            >
                              恢复
                            </button>
                          ) : (
                            <button
                              className="btn btn--sm"
                              onClick={() => void tokenAction("blacklist", t)}
                            >
                              黑名单
                            </button>
                          )}
                          <button
                            className="btn btn--sm"
                            onClick={() => void tokenAction("remove", t)}
                          >
                            删除
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            <div className={`row mt-12${tokensOpen ? "" : " is-hidden"}`}>
              <input
                placeholder="符号，如 LINK"
                value={tok.symbol}
                onChange={(e) => setTok({ ...tok, symbol: e.target.value })}
                style={{ height: 30, padding: "0 8px", minWidth: 120 }}
              />
              <input
                placeholder="0x… 地址"
                value={tok.address}
                onChange={(e) => setTok({ ...tok, address: e.target.value })}
                style={{ height: 30, padding: "0 8px", flex: 1, minWidth: 260 }}
              />
              <input
                type="number"
                value={tok.decimals}
                onChange={(e) => setTok({ ...tok, decimals: e.target.value })}
                style={{ height: 30, width: 80, padding: "0 8px" }}
              />
              <button className="btn btn--sm" onClick={() => void addToken()}>
                加入白名单
              </button>
            </div>
            <p className="hint mt-8">{cfg?.note}</p>
            </>
            )}
          </>
        ) : (
          <p className="hint">加载搜索配置…</p>
        )}
      </div>
    </section>
  );
}
