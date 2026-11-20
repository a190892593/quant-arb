import { useState } from "react";
import { data } from "../data";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";
import { DiscoveryPanel } from "../components/DiscoveryPanel";

type ChipVariant = "ok" | "bad" | "warn" | "accent" | "idle";

const POOL_VARIANT: Record<string, ChipVariant> = { ENABLED: "ok", OBSERVE: "warn", BLOCKED: "bad" };
const TOKEN_VARIANT: Record<string, ChipVariant> = { ALLOWED: "ok", OBSERVE: "warn", BLOCKED: "bad" };

/** 触发告警样式的风险标签（与原型同一表达式） */
const RISK_TAG = /Fee-on|非标准|桥接|待验证/;

type PoolFilter = "ALL" | "ENABLED" | "OBSERVE" | "BLOCKED";

const FILTERS: readonly (readonly [PoolFilter, string])[] = [
  ["ALL", "全部"],
  ["ENABLED", "启用"],
  ["OBSERVE", "观察"],
  ["BLOCKED", "阻断"],
];

export function Pools() {
  const [poolFilter, setPoolFilter] = useState<PoolFilter>("ALL");
  const { pools: allPools, tokens, adapters } = data;

  const pools = allPools.filter((p) => poolFilter === "ALL" || p.state === poolFilter);
  const allowed = tokens.filter((t) => t.state === "ALLOWED").length;
  const observe = allPools.filter((p) => p.state === "OBSERVE").length;
  const blocked = allPools.filter((p) => p.state === "BLOCKED").length;
  const tvl = allPools.reduce((s, p) => s + p.tvl, 0);
  const vol = allPools.reduce((s, p) => s + p.vol24, 0);

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow"><span className="idx">07</span> RESEARCH · REGISTRY</div>
          <h1>池与 Token 注册表</h1>
          <p className="page-sub">协议适配器、池、Token 三层白名单。未验证 Token、特殊 Hook、新池默认不进入执行白名单，先进观察与模拟。</p>
        </div>
        <div className="page-actions">
          <button
            className="btn"
            onClick={() => toast("代码哈希已同步", { sub: "6 个适配器哈希一致，无变更" })}
          >
            <Icon name="refresh" /> 同步链上代码哈希
          </button>
          <button
            className="btn"
            onClick={() =>
              toast("新池进入观察队列", { sub: "需完成池状态模拟与流动性校验后才会进入白名单", variant: "warn" })
            }
          >
            <Icon name="pools" /> 登记新池
          </button>
        </div>
      </div>

      <DiscoveryPanel />

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">跟踪池<span className="unit">POOLS</span></div>
            <div className="kpi-value">326</div>
            <div className="kpi-foot"><span>本页展示 {allPools.length} 个主池 · 6 协议</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">TVL / 24h 量<span className="unit">USDC</span></div>
            <div className="kpi-value">{fmt.compact(tvl)}</div>
            <div className="kpi-foot"><span>24h {fmt.compact(vol)}</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">执行白名单<span className="unit">TOKENS</span></div>
            <div className="kpi-value">{allowed}<small>/ {tokens.length}</small></div>
            <div className="kpi-foot">
              <Chip variant="warn">观察 {tokens.filter((t) => t.state === "OBSERVE").length}</Chip>
              <Chip variant="bad">阻断 {tokens.filter((t) => t.state === "BLOCKED").length}</Chip>
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-label">异常池<span className="unit">OBSERVE + BLOCKED</span></div>
            <div className="kpi-value">{observe + blocked}</div>
            <div className="kpi-foot"><span>观察 {observe} · 阻断 {blocked}</span></div>
          </div>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>池注册表 <span className="count">{pools.length} / {allPools.length}</span></h2>
            <div className="panel-meta">
              <div className="seg">
                {FILTERS.map(([key, label]) => (
                  <button
                    key={key}
                    className={poolFilter === key ? "is-active" : ""}
                    onClick={() => setPoolFilter(key)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>池地址</th>
                    <th>协议</th>
                    <th>交易对</th>
                    <th className="is-num">费率</th>
                    <th>链</th>
                    <th className="is-num">TVL</th>
                    <th className="is-num">24h 量</th>
                    <th>适配器</th>
                    <th className="is-num">状态年龄</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {pools.map((p) => (
                    <tr key={p.addr}>
                      <td className="cell-id">{p.addr}</td>
                      <td className="cell-strong">{p.protocol}</td>
                      <td className="mono">{p.pair}</td>
                      <td className="is-num muted">{p.fee}</td>
                      <td>{p.chain}</td>
                      <td className="is-num">{p.tvl ? fmt.compact(p.tvl) : "—"}</td>
                      <td className="is-num muted">{p.vol24 ? fmt.compact(p.vol24) : "—"}</td>
                      <td>
                        {p.adapter === "—" ? (
                          <span className="faint">—</span>
                        ) : (
                          <span className="tag-mini">{p.adapter}</span>
                        )}
                      </td>
                      <td className="is-num muted">{p.age}</td>
                      <td>
                        <Chip variant={POOL_VARIANT[p.state] ?? "idle"} dot>{p.state}</Chip>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">新池先进入观察与模拟，成交可信度达标后才进入执行白名单；Pool / Router 代码或版本变化会立即暂停相关适配器。</div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>Token 白名单 <span className="count">{tokens.length}</span></h2>
            <div className="panel-meta">起始资产 / 中间资产 / 闪电贷 分别授权</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>资产</th>
                    <th>链</th>
                    <th>地址</th>
                    <th className="is-num">精度</th>
                    <th>代码哈希</th>
                    <th>风险标签</th>
                    <th>起始</th>
                    <th>中间</th>
                    <th>闪电贷</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {tokens.map((t) => (
                    <tr key={t.addr}>
                      <td className="cell-strong">{t.sym}</td>
                      <td className="muted">{t.chain}</td>
                      <td className="cell-id">{t.addr}</td>
                      <td className="is-num">{t.dec}</td>
                      <td className="cell-id">{t.hash}</td>
                      <td>
                        <div className="tags">
                          {t.tags.map((x) => (
                            <span key={x} className={`tag-mini${RISK_TAG.test(x) ? " tag-mini--warn" : ""}`}>{x}</span>
                          ))}
                        </div>
                      </td>
                      <td>{t.start ? <span className="flag-on">✓</span> : <span className="flag-off">—</span>}</td>
                      <td>{t.mid ? <span className="flag-on">✓</span> : <span className="flag-off">—</span>}</td>
                      <td>{t.flash ? <span className="flag-on">✓</span> : <span className="flag-off">—</span>}</td>
                      <td>
                        <Chip variant={TOKEN_VARIANT[t.state] ?? "idle"} dot>{t.state}</Chip>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">默认拒绝未验证的 Fee-on-Transfer、Rebasing、可暂停/冻结或代理逻辑可变的 Token；确需支持时必须独立适配与测试。</div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>适配器注册表 <span className="count">{adapters.length}</span></h2>
            <div className="panel-meta">变更属于高风险管理操作</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>协议</th>
                    <th>链</th>
                    <th>版本</th>
                    <th>Factory</th>
                    <th>Router</th>
                    <th>代码哈希</th>
                    <th>审计</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {adapters.map((a) => (
                    <tr key={`${a.protocol}-${a.chain}`}>
                      <td className="cell-strong">{a.protocol}</td>
                      <td>{a.chain}</td>
                      <td><span className="tag-mini">{a.version}</span></td>
                      <td className="cell-id">{a.factory}</td>
                      <td className="cell-id">{a.router}</td>
                      <td className="cell-id">{a.hash}</td>
                      <td className="mono muted" style={{ fontSize: 10 }}>{a.audit}</td>
                      <td>
                        <Chip variant={a.state === "ENABLED" ? "ok" : "bad"} dot>{a.state}</Chip>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">Adapter 必须登记地址、Factory、版本与代码哈希；V4 / Hook 未独立审计前不得进入执行路径。</div>
        </section>
      </div>
    </>
  );
}
