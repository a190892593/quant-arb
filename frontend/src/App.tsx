import { useEffect, useState, type JSX } from "react";
import { Topbar, Rail } from "./components/Shell";
import { OppDrawer } from "./components/OppDrawer";
import { CommandPalette } from "./components/CommandPalette";
import { Toaster } from "./ui/Toast";
import { data, startPolling, useDataStatus } from "./data";
import { Overview } from "./views/Overview";
import { RouteSearch } from "./views/RouteSearch";
import { Executions } from "./views/Executions";
import { Accounts } from "./views/Accounts";
import { Performance } from "./views/Performance";
import { Strategy } from "./views/Strategy";
import { Pools } from "./views/Pools";
import { Audit } from "./views/Audit";
import { DataQuality } from "./views/DataQuality";
import { Ledger } from "./views/Ledger";
import { Risk } from "./views/Risk";
import { Contract } from "./views/Contract";
import { System } from "./views/System";
import { Capability } from "./views/Capability";
import { Pending } from "./views/Pending";
import { DataSources } from "./views/DataSources";

const ROUTES: Record<string, () => JSX.Element> = {
  overview: Overview,
  scannerDex: RouteSearch,
  executions: Executions,
  accounts: Accounts,
  performance: Performance,
  strategy: Strategy,
  pools: Pools,
  audit: Audit,
  dataquality: DataQuality,
  ledger: Ledger,
  risk: Risk,
  contract: Contract,
  system: System,
  sources: DataSources,
  capability: Capability,
};

/** url segment → view key (nav ids are camelCase, urls are kebab-case) */
const URL_ROUTE: Record<string, string> = {
  overview: "overview",
  "scanner-dex": "scannerDex",
  routes: "scannerDex",
  scanner: "scannerDex",
  "scanner-cex": "scannerDex", // CEX 扫描已移除，旧链接重定向到 DEX 搜索引擎
  executions: "executions",
  accounts: "accounts",
  performance: "performance",
  strategy: "strategy",
  pools: "pools",
  audit: "audit",
  dataquality: "dataquality",
  ledger: "ledger",
  risk: "risk",
  contract: "contract",
  system: "system",
  sources: "sources",
  capability: "capability",
};

function useHashRoute(): string {
  const read = () => location.hash.replace(/^#\/?/, "") || "overview";
  const [route, setRoute] = useState(read);
  useEffect(() => {
    const onHash = () => setRoute(read());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);
  return route;
}

export function App() {
  const route = useHashRoute();
  const [name, param] = route.split("/");
  const status = useDataStatus();
  const [cmdOpen, setCmdOpen] = useState(false);

  // 实时数据轮询（幂等；StrictMode 下不会重复启动）
  useEffect(() => {
    startPolling();
  }, []);

  // 命令面板快捷键
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCmdOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const oppId = name === "opportunity" ? param : undefined;
  const opp = oppId ? data.opps.find((o) => o.id === oppId) : undefined;
  const routeKey = URL_ROUTE[name] ?? name;
  const base = opp ? "scannerDex" : routeKey;
  const active = oppId ? base : name === "strategy" ? `strategy:${param}` : routeKey;
  const Page = ROUTES[routeKey] ?? (opp ? ROUTES[base] : undefined);
  const showPending = !oppId && !ROUTES[routeKey];

  const scannerState = data.scanner.state;
  const running = scannerState === "RUNNING";
  const online = data.meta?.online ?? false;
  const block = data.chains.find((c) => c.block > 0)?.block ?? 0;

  return (
    <div className="app" data-live-version={status.version}>
      <Topbar
        running={running}
        online={online}
        scannerState={scannerState}
        block={block}
        onCommand={() => setCmdOpen(true)}
      />
      <Rail active={active} />
      <main className="stage" tabIndex={-1}>
        <div className="view">{showPending ? <Pending route={route} /> : Page ? <Page /> : null}</div>
        <footer className="stage-foot">
          <span>ARBX 第一阶段 · 本地模拟环境 · 不接入实盘签名器</span>
          <span>净收益口径：实际回款 − 实际投入 − 全部执行成本</span>
        </footer>
      </main>
      {opp && (
        <OppDrawer
          id={opp.id}
          onClose={() => {
            history.replaceState(null, "", "#/scanner-dex");
            window.dispatchEvent(new HashChangeEvent("hashchange"));
          }}
        />
      )}
      <CommandPalette open={cmdOpen} onClose={() => setCmdOpen(false)} />
      <Toaster />
    </div>
  );
}
