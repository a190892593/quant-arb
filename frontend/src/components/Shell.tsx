import { useEffect, useState } from "react";
import { data } from "../data";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { NAV } from "./nav";

export function Topbar({
  running,
  online,
  scannerState,
  block,
  onCommand,
}: {
  running: boolean;
  online: boolean;
  scannerState: string;
  block: number;
  onCommand: () => void;
}) {
  const [clock, setClock] = useState(() => fmt.clock(new Date()));
  useEffect(() => {
    const t = setInterval(() => setClock(fmt.clock(new Date())), 1000);
    return () => clearInterval(t);
  }, []);

  const health = data.health.slice(0, 3);

  return (
    <header className="topbar">
      <a className="brand" href="#/overview" aria-label="ARBX 总览">
        <svg className="brand-glyph" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M3 20 21 4" stroke="currentColor" strokeWidth="1.4" />
          <path d="M3 4 21 20" stroke="currentColor" strokeWidth="1.4" opacity=".38" />
          <circle cx="12" cy="12" r="2.6" fill="none" stroke="currentColor" strokeWidth="1.4" />
        </svg>
        <span className="brand-text">
          <b>ARBX</b>
          <i>ATOMIC ARBITRAGE TERMINAL</i>
        </span>
      </a>

      <div className="topbar-mid">
        <span className="tag tag--paper" title="第一阶段：本地模拟，不发送任何真实交易">PAPER</span>
        <span
          className={`tag ${online ? "tag--live" : "tag--demo"}`}
          title={online ? "数据来自本地 arb-backend（Paper 模式）" : "后端不可达，展示演示数据"}
        >
          {online ? "LIVE API" : "DEMO · OFFLINE"}
        </span>
        <span className="tsep" aria-hidden="true" />
        <span className="datum"><i>本地时间</i><b>{clock}</b></span>
        <span className="datum"><i>链头 · ARB</i><b>{block ? fmt.num(block, 0) : "—"}</b></span>
        <span className="datum">
          <i>扫描</i>
          <b className={running ? "is-live" : ""}>{scannerState || "STOPPED"}</b>
        </span>
      </div>

      <div className="topbar-right">
        <div className="health" aria-label="数据源健康">
          {health.map((h) => (
            <span className="health-item" key={h.key} title={h.note}>
              {h.key}
              <b>{h.value}</b>
              <span className={`dot ${h.value === "UP" ? "dot--ok" : h.value === "DEGRADED" ? "dot--warn" : "dot--idle"}`} />
            </span>
          ))}
        </div>
        <button className="btn btn--ghost btn--icon" onClick={onCommand} title="命令面板 (Ctrl/Cmd + K)" aria-label="命令面板">
          <Icon name="search" size={14} />
        </button>
      </div>
    </header>
  );
}

const pctOf = (cur: number, limit: number): number => {
  if (!limit) return 0;
  return Math.max(0, Math.min(999, (cur / limit) * 100));
};

export function Rail({ active }: { active: string }) {
  const rs = data.riskStatus;
  const paused = rs?.globalPaused ?? false;
  const equity = rs?.equity || data.equity || 5000;

  const maxExposure = data.executions.length
    ? Math.max(0, ...data.executions.map((e) => e.exposure))
    : 0;

  const guards = rs
    ? [
        { label: "日亏损", cur: pctOf(rs.accountDailyLoss, equity * 0.01) },
        { label: "回撤", cur: pctOf(rs.maxDrawdown, equity * 0.05) },
        { label: "敞口", cur: pctOf(maxExposure, equity * 0.005) },
      ]
    : [
        { label: "日亏损", cur: 24.8 },
        { label: "回撤", cur: 19.6 },
        { label: "敞口", cur: 13.6 },
      ];

  const actionableDex = data.opps.filter((o) => o.strategy !== "cex_dex" && o.status === "ACTIONABLE").length;
  const actionableCex = data.opps.filter((o) => o.strategy === "cex_dex" && o.status === "ACTIONABLE").length;

  return (
    <aside className="rail">
      <nav className="nav" aria-label="主导航">
        {NAV.map((group) => (
          <div className="nav-group" key={group.label}>
            <div className="nav-label">{group.label}</div>
            {group.items.map((item) => {
              const badge =
                item.id === "scannerDex" ? actionableDex : item.id === "scannerCex" ? actionableCex : undefined;
              return (
                <a
                  key={item.id}
                  className={`nav-item${active === item.id ? " is-active" : ""}`}
                  href={item.hash}
                  data-nav={item.id}
                >
                  <Icon name={item.icon} />
                  <span className="nav-text">{item.label}</span>
                  {badge !== undefined && badge > 0 && <span className="nav-badge">{badge}</span>}
                </a>
              );
            })}
          </div>
        ))}
      </nav>

      <div className="rail-foot">
        <div className={`risk-spine${paused ? " is-paused" : ""}`}>
          <div className="risk-spine-head">
            <span className="eyebrow">GLOBAL RISK</span>
            <span className={`dot ${paused ? "dot--bad" : "dot--ok"}`} />
          </div>
          <div className="risk-spine-state">{paused ? "PAUSED" : "ARMED"}</div>
          <div className="risk-spine-rows">
            {guards.map((g) => (
              <div className="rs-row" key={g.label}>
                <div className="rs-row-top">
                  <span>{g.label}</span>
                  <b>{fmt.pct(g.cur, 0)}</b>
                </div>
                <div className={`bar${g.cur >= 85 ? " bar--bad" : g.cur >= 60 ? " bar--warn" : ""}`} style={{ height: 3 }}>
                  <i style={{ width: `${Math.min(100, g.cur)}%` }} />
                </div>
              </div>
            ))}
          </div>
        </div>
        <div className="rail-version">
          <span>{rs?.paramVersion ? rs.paramVersion.replace("risk-", "") : "v1.2 · TDD"}</span>
          <span>{rs ? "LIVE" : "LOCAL FIRST"}</span>
        </div>
      </div>
    </aside>
  );
}
