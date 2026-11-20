import { useEffect, useState } from "react";
import type { SearchConfigResponse, SearchParamsConfig, SearchTokenEntry } from "../types";
import { fetchSearchConfig, saveSearchConfig, updateSearchToken } from "../data/api";
import { Chip } from "../ui/Chip";
import { Icon } from "../lib/icons";
import { toast } from "../ui/Toast";
import { Drawer } from "../ui/Drawer";

type Field = {
  key: keyof SearchParamsConfig;
  label: string;
  unit?: string;
  step: number;
  min?: number;
};

type Section = { id: string; title: string; hint: string; fields: Field[] };

const SECTIONS: Section[] = [
  {
    id: "admit",
    title: "池与准入",
    hint: "决定哪些池能进入搜索图，以及单池能吃多深",
    fields: [
      { key: "min_pool_tvl_usd", label: "池 TVL 门槛", unit: "USD", step: 500, min: 0 },
      { key: "max_pool_share_pct", label: "单池深度占比", unit: "%", step: 1, min: 0 },
      { key: "max_pool_state_age_ms", label: "池状态最大年龄", unit: "ms", step: 1000, min: 0 },
      { key: "max_pools", label: "参与池上限", step: 1, min: 1 },
    ],
  },
  {
    id: "size",
    title: "规模与跳数",
    hint: "投入区间、跳数范围与投入优化网格",
    fields: [
      { key: "min_input_usd", label: "单笔下限", unit: "USD", step: 50, min: 1 },
      { key: "max_input_usd", label: "单笔上限", unit: "USD", step: 500, min: 1 },
      { key: "min_hops", label: "最小跳数", step: 1, min: 2 },
      { key: "max_hops", label: "最大跳数", step: 1, min: 2 },
      { key: "grid_points", label: "投入网格点数", step: 5, min: 5 },
      { key: "refine_iters", label: "最优点细化迭代", step: 1, min: 1 },
    ],
  },
  {
    id: "budget",
    title: "搜索预算",
    hint: "单轮枚举与定价的算力上限，防止一轮拖垮扫描",
    fields: [
      { key: "max_candidates", label: "候选路线上限", step: 1000, min: 0 },
      { key: "max_evaluate", label: "单轮定价上限", step: 50, min: 1 },
      { key: "max_evaluate_ms", label: "单轮定价预算", unit: "ms", step: 100, min: 50 },
    ],
  },
  {
    id: "fresh",
    title: "时效与增量",
    hint: "机会有效期、报价复用与全量重建节奏",
    fields: [
      { key: "ttl_sec", label: "机会有效期", unit: "s", step: 5, min: 5 },
      { key: "requote_age_ms", label: "报价复用上限", unit: "ms", step: 500, min: 500 },
      { key: "full_rescan_blocks", label: "全量重建间隔", unit: "块", step: 24, min: 1 },
    ],
  },
  {
    id: "cost",
    title: "成本与安全垫",
    hint: "这些假设直接决定「净利」口径，改动会同步影响历史口径",
    fields: [
      { key: "gas_price_gwei", label: "Gas 价格覆盖", unit: "gwei · 0=链上", step: 0.01, min: 0 },
      { key: "priority_fee_usd", label: "优先费", unit: "USD", step: 0.01, min: 0 },
      { key: "flash_fee_bps", label: "闪电贷费率", unit: "bps", step: 1, min: 0 },
      { key: "delay_reserve_bps", label: "延迟预留", unit: "bps", step: 0.5, min: 0 },
      { key: "mev_reserve_bps", label: "MEV 预留", unit: "bps", step: 0.5, min: 0 },
      { key: "profit_discount_pct", label: "利润折扣", unit: "%", step: 1, min: 0 },
    ],
  },
  {
    id: "verify",
    title: "一致性与压力",
    hint: "M1/M2 偏差阈值与 M3 压力场景的扰动幅度",
    fields: [
      { key: "model_mismatch_max_bps", label: "M1/M2 偏差阈值", unit: "bps", step: 5, min: 1 },
      { key: "stress_adverse_bps", label: "池状态扰动", unit: "bps", step: 5, min: 0 },
      { key: "stress_gas_increase_pct", label: "压力 Gas 上调", unit: "%", step: 10, min: 0 },
    ],
  },
];

export function SearchConfigDrawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [cfg, setCfg] = useState<SearchConfigResponse | null>(null);
  const [draft, setDraft] = useState<SearchParamsConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [ladderText, setLadderText] = useState("");
  const [tok, setTok] = useState({ symbol: "", address: "", decimals: "18" });
  const [tokensOpen, setTokensOpen] = useState(false);

  const load = async () => {
    try {
      const c = await fetchSearchConfig();
      setCfg(c);
      setDraft(c.params);
      setLadderText((c.params.notional_ladder ?? []).join(", "));
      setErr(null);
    } catch (e) {
      setErr(String(e));
    }
  };

  useEffect(() => {
    if (open) void load();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const patch = (key: keyof SearchParamsConfig, value: number) =>
    setDraft((d) => (d ? { ...d, [key]: value } : d));

  const save = async () => {
    if (!draft || busy) return;
    setBusy(true);
    try {
      const ladder = ladderText
        .split(/[,，\s]+/)
        .map((s) => Number(s))
        .filter((n) => Number.isFinite(n) && n > 0);
      await saveSearchConfig({ params: { ...draft, notional_ladder: ladder } });
      await load();
      toast("搜索参数已保存并即时生效", { variant: "info" });
    } catch (e) {
      toast("保存搜索参数失败", { sub: String(e), variant: "bad" });
    } finally {
      setBusy(false);
    }
  };

  const tokenAction = async (action: "whitelist" | "blacklist" | "remove", t: SearchTokenEntry) => {
    try {
      await updateSearchToken({ action, symbol: t.symbol, address: t.address, decimals: t.decimals });
      await load();
      toast(`${t.symbol} 已${action === "remove" ? "移除" : action === "blacklist" ? "加入黑名单" : "加入白名单"}`);
    } catch (e) {
      toast("Token 操作失败", { sub: String(e), variant: "bad" });
    }
  };

  const addToken = async () => {
    if (!tok.symbol.trim()) return;
    try {
      await updateSearchToken({
        action: "whitelist",
        symbol: tok.symbol.trim(),
        address: tok.address.trim(),
        decimals: Number(tok.decimals) || 18,
      });
      setTok({ symbol: "", address: "", decimals: "18" });
      await load();
      toast("Token 已加入白名单", { variant: "info" });
    } catch (e) {
      toast("添加 Token 失败", { sub: String(e), variant: "bad" });
    }
  };

  if (!open) return null;

  const nums = (draft ?? {}) as unknown as Record<string, number | undefined>;

  return (
    <>
      <div className="drawer-scrim" onClick={onClose} />
      <Drawer
        ariaLabel="搜索配置"
        storageKey="search-config"
        onClose={onClose}
        head={
          <>
            <div className="drawer-id">搜索配置</div>
            <div className="drawer-sub">
              {cfg ? `白名单 ${cfg.whitelist} · 黑名单 ${cfg.blacklist}` : "LOADING"}
              <span className="sep" />
              <span>参数保存后即时生效，下一轮搜索即采用</span>
            </div>
          </>
        }
        footer={
          <>
            <span className="hint">参数保存后即时生效；Token 名单变更需重启后端重建 feed。</span>
            <div className="row">
              <button className="btn btn--sm" onClick={() => void load()} disabled={busy}>
                <Icon name="refresh" />
                重载
              </button>
              <button className="btn btn--ghost" onClick={onClose}>
                关闭
              </button>
              <button className="btn btn--primary" onClick={() => void save()} disabled={busy || !draft}>
                {busy ? "保存中…" : "保存配置"}
              </button>
            </div>
          </>
        }
      >
          {err && <div className="route-offline">读取搜索配置失败：{err}</div>}
          {!draft && !err && <p className="hint">加载配置…</p>}

          {draft && (
            <>
              {SECTIONS.map((s) => (
                <section className="cfg-sec" key={s.id}>
                  <div className="cfg-sec-head">
                    <h4>{s.title}</h4>
                    <span>{s.hint}</span>
                  </div>
                  <div className="cfg-grid">
                    {s.fields.map((f) => (
                      <label className="cfg-field" key={f.key}>
                        <span className="cfg-label">
                          {f.label}
                          {f.unit && <em>{f.unit}</em>}
                        </span>
                        <input
                          type="number"
                          step={f.step}
                          min={f.min}
                          value={nums[f.key] ?? 0}
                          onChange={(e) => patch(f.key, Number(e.target.value))}
                        />
                      </label>
                    ))}
                  </div>
                </section>
              ))}

              <section className="cfg-sec">
                <div className="cfg-sec-head">
                  <h4>投入阶梯</h4>
                  <span>逗号分隔，逐档求解最优规模；留空则用默认阶梯</span>
                </div>
                <input
                  className="cfg-text"
                  value={ladderText}
                  onChange={(e) => setLadderText(e.target.value)}
                  placeholder="100, 250, 400, 500, 1000, 2500, 5000"
                />
              </section>

              <section className="cfg-sec">
                <div className="cfg-sec-head">
                  <button className="cfg-toggle" onClick={() => setTokensOpen((v) => !v)} aria-expanded={tokensOpen}>
                    <h4>
                      Token 白 / 黑名单 <span className="count">{cfg?.tokens.length ?? 0}</span>
                    </h4>
                    <b className={`chev${tokensOpen ? " is-open" : ""}`}>▾</b>
                  </button>
                  <span>未登记的 Token 不进入搜索图</span>
                </div>
                {tokensOpen && (
                  <>
                    <div className="tbl-wrap">
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
                              <td className="mono">{t.address ? `${t.address.slice(0, 8)}…${t.address.slice(-6)}` : "—"}</td>
                              <td>
                                <div className="row">
                                  {t.status === "BLACKLIST" ? (
                                    <button className="btn btn--sm" onClick={() => void tokenAction("whitelist", t)}>
                                      恢复
                                    </button>
                                  ) : (
                                    <button className="btn btn--sm btn--ghost" onClick={() => void tokenAction("blacklist", t)}>
                                      黑名单
                                    </button>
                                  )}
                                  <button className="btn btn--sm btn--ghost" onClick={() => void tokenAction("remove", t)}>
                                    删除
                                  </button>
                                </div>
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                    <div className="row mt-12">
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
                        style={{ height: 30, padding: "0 8px", flex: 1, minWidth: 240 }}
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
                  </>
                )}
              </section>
            </>
          )}
      </Drawer>
    </>
  );
}
