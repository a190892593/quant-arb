import { useEffect, useState } from "react";
import type { SourceConfig, SourceEntry } from "../types";
import { fetchSources, saveSources, applySourcePreset } from "../data/api";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

const STRATEGIES: { value: string; label: string; desc: string }[] = [
  { value: "round_robin", label: "轮询", desc: "每轮换下一个启用的数据源，负载最均匀" },
  { value: "priority", label: "优先级", desc: "始终从 priority 最小（数字越小越优先）的源开始，失败再降级" },
  { value: "random", label: "随机", desc: "每轮随机起点，适合多源均摊限流" },
];

const KIND_LABEL: Record<string, string> = {
  gecko: "DEX 聚合器",
  coingecko: "DEX 聚合器",
  dexscreener: "DEX 聚合器",
  data: "TVL / 数据",
  security: "安全检测",
  explorer: "浏览器",
  rpc: "RPC",
  cex: "CEX 行情",
};

export function DataSources() {
  const [cfg, setCfg] = useState<SourceConfig | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    try {
      setCfg(await fetchSources());
    } catch {
      /* ignore */
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const patch = (id: string, field: keyof SourceEntry, value: string | number | boolean) => {
    setCfg((c) =>
      c
        ? {
            ...c,
            sources: c.sources.map((s) => (s.id === id ? { ...s, [field]: value } : s)),
          }
        : c,
    );
  };

  const preset = async (mode: "free" | "boost") => {
    if (busy) return;
    setBusy(true);
    try {
      const r = await applySourcePreset(mode);
      setCfg(r.config);
      toast(mode === "free" ? "已切换到免费模式" : "已切换到增强模式", {
        variant: "info",
        sub: r.warnings.length ? r.warnings.join("；") : "配置已保存并生效",
      });
    } catch (e) {
      toast("切换预设失败", { sub: String(e), variant: "bad" });
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {    if (!cfg || busy) return;
    setBusy(true);
    try {
      const saved = await saveSources(cfg);
      setCfg(saved);
      toast("数据源配置已保存并生效", { variant: "info" });
    } catch (e) {
      toast("保存数据源配置失败", { sub: String(e), variant: "bad" });
    } finally {
      setBusy(false);
    }
  };

  const enabled = cfg?.sources.filter((s) => s.enabled).length ?? 0;

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">SCAN</span> DATA SOURCES
          </div>
          <h1>扫描数据源配置</h1>
          <p className="page-sub">
            管理发现任务的候选数据源、Key、限速、优先级与切换策略。保存后下一轮发现立即生效。
          </p>
        </div>
        <div className="page-actions">
          <button className="btn" onClick={() => void preset("free")} disabled={busy}>
            免费模式
          </button>
          <button className="btn" onClick={() => void preset("boost")} disabled={busy}>
            增强模式
          </button>
          <button className="btn" onClick={() => void load()} disabled={busy}>
            重新加载
          </button>
          <button className="btn" onClick={() => void save()} disabled={busy}>
            {busy ? "保存中…" : "保存配置"}
          </button>
        </div>
      </div>

      <section className="panel mt-12">
        <header className="panel-head">
          <h2>
            切换策略 <span className="count">STRATEGY</span>
          </h2>
          <div className="panel-meta">
            {enabled} 启用 ·{" "}
            {cfg?.sources.some((s) => s.enabled && s.api_key) ? "增强模式" : "免费模式"}
          </div>
        </header>
        <div className="panel-body">
          {cfg ? (
            <>
              <div className="row">
                <label className="cfg-field">
                  <span>策略</span>
                  <select
                    value={cfg.strategy}
                    onChange={(e) => setCfg({ ...cfg, strategy: e.target.value })}
                    style={{ height: 30, padding: "0 8px", minWidth: 160 }}
                  >
                    {STRATEGIES.map((s) => (
                      <option key={s.value} value={s.value}>
                        {s.label}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="cfg-field">
                  <span>发现间隔 (秒)</span>
                  <input
                    type="number"
                    min={60}
                    step={60}
                    value={cfg.interval_sec}
                    onChange={(e) =>
                      setCfg({ ...cfg, interval_sec: Math.max(60, Number(e.target.value) || 60) })
                    }
                    style={{ height: 30, padding: "0 8px", width: 140 }}
                  />
                </label>
                <span className="hint">
                  {STRATEGIES.find((s) => s.value === cfg.strategy)?.desc}
                </span>
              </div>
            </>
          ) : (
            <p className="hint">加载数据源配置…</p>
          )}
        </div>
      </section>

      <section className="panel mt-12">
        <header className="panel-head">
          <h2>
            数据源列表 <span className="count">SOURCES</span>
          </h2>
        </header>
        <div className="panel-body">
          <div className="tbl-wrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>启用</th>
                  <th>数据源</th>
                  <th>类型</th>
                  <th>优先级</th>
                  <th>Key</th>
                  <th>限速/分</th>
                  <th>最近状态</th>
                  <th>最近使用</th>
                </tr>
              </thead>
              <tbody>
                {(cfg?.sources ?? []).map((s) => (
                  <tr key={s.id}>
                    <td>
                      <input
                        type="checkbox"
                        checked={s.enabled}
                        onChange={(e) => patch(s.id, "enabled", e.target.checked)}
                      />
                    </td>
                    <td>
                      <b>{s.name}</b>
                      <div className="mono" style={{ fontSize: 11, opacity: 0.65 }}>
                        {s.id}
                      </div>
                    </td>
                    <td>{KIND_LABEL[s.kind] ?? s.kind}</td>
                    <td>
                      <input
                        type="number"
                        value={s.priority}
                        onChange={(e) => patch(s.id, "priority", Number(e.target.value))}
                        style={{ width: 64, height: 26, padding: "0 6px" }}
                      />
                    </td>
                    <td>
                      <input
                        value={s.api_key}
                        placeholder={s.kind === "coingecko" ? "CG-..." : "无需 Key / 可填"}
                        onChange={(e) => patch(s.id, "api_key", e.target.value)}
                        style={{ width: 180, height: 26, padding: "0 6px" }}
                      />
                    </td>
                    <td>{s.rate_limit_per_min}</td>
                    <td>
                      {s.last_status ? (
                        <Chip variant={s.last_status === "ok" ? "ok" : "warn"}>{s.last_status}</Chip>
                      ) : (
                        <span className="muted">—</span>
                      )}
                    </td>
                    <td className="mono" style={{ fontSize: 11 }}>
                      {s.last_used_at ? s.last_used_at.replace("T", " ").slice(0, 19) : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="hint mt-8">
            Key 会脱敏显示；留空或保持 <code>****</code> 表示不修改，填写新值才会覆盖。Key
            保存在本机 <code>data/source_config.json</code>，不会上传。
          </p>
        </div>
      </section>
    </>
  );
}
