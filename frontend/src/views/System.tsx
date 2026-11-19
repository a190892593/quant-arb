import { data } from "../data";
import type { DataModel } from "../types";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

/* The demo dataset still carries the system-registry tables; the typed facade
   does not expose them yet, so this view declares the slice it reads. */
interface ServiceRow {
  name: string;
  role: string;
  state: string;
  uptime: string;
  p95: string;
  note: string;
}

interface ExchangeRow {
  name: string;
  state: string;
  adapter: string;
  bookAge: string;
  gaps: number;
}

interface DexProtocolRow {
  name: string;
  type: string;
  pools: number;
  chains: number;
  state: string;
  adapter: string;
}

interface SystemSlice {
  services: ServiceRow[];
  exchanges: ExchangeRow[];
  dexProtocols: DexProtocolRow[];
  CHAIN_STAGES: string[];
}

const d = data as unknown as DataModel & SystemSlice;

/** TDD §15 · 本机重启恢复检查，顺序执行 */
const RESTART_STEPS: [string, string, "ok" | "warn"][] = [
  ["恢复 PostgreSQL", "连接池就绪 · 迁移版本一致", "ok"],
  ["校验账本与配置", "账本差异 0.00 · 配置 cfg-2026.11.19-b4", "ok"],
  ["重新同步链头", "Arbitrum / Base 区块一致，Polygon 暂停", "ok"],
  ["重新同步 CEX 订单簿 Snapshot", "ETH/USDT 完整 · WBTC/USDT 等待重同步", "warn"],
  ["检查未完成 Paper 交易", "1 笔状态未知已核对，1 笔人工处置中", "warn"],
  ["重新初始化扫描任务", "扫描器已启动，3 链 7 协议", "ok"],
  ["数据完整前禁止生成机会", "闸门生效：盘口不完整的交易对不产出机会", "ok"],
];

export function System() {
  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">13</span> GOVERN · SYSTEM
          </div>
          <h1>系统</h1>
          <p className="page-sub">
            本地优先部署：不使用容器，全部服务以本机进程运行。链与交易所按准入等级放行，未达等级的链路不产生可执行机会。
          </p>
        </div>
        <div className="page-actions">
          <span className="chip chip--ok">
            <span className="dot dot--ok" />
            HEALTHY
          </span>
          <button
            className="btn"
            onClick={() =>
              toast("健康检查完成：5 服务 UP", { sub: "RPC P95 188 ms · 盘口 42 ms · 账本差异 0.00" })
            }
          >
            <Icon name="refresh" /> 运行健康检查
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="grid g-4">
          <section className="panel">
            <div className="panel-body">
              <div className="eyebrow">SERVICES</div>
              <div className="mono mt-8" style={{ fontSize: 22 }}>
                5 <span className="muted" style={{ fontSize: 11 }}>/ 5 UP</span>
              </div>
              <div className="hint mt-8">arb-api · arb-worker · arb-paper · postgres</div>
            </div>
          </section>
          <section className="panel">
            <div className="panel-body">
              <div className="eyebrow">CHAIN HEAD</div>
              <div className="mono mt-8" style={{ fontSize: 22 }}>
                0 <span className="muted" style={{ fontSize: 11 }}>BLOCK LAG</span>
              </div>
              <div className="hint mt-8">Arbitrum / Base 双 RPC 哈希一致</div>
            </div>
          </section>
          <section className="panel">
            <div className="panel-body">
              <div className="eyebrow">CEX BOOK AGE</div>
              <div className="mono mt-8" style={{ fontSize: 22 }}>
                42 <span className="muted" style={{ fontSize: 11 }}>ms</span>
              </div>
              <div className="hint mt-8">阈值 250 ms · 序列缺口 0</div>
            </div>
          </section>
          <section className="panel">
            <div className="panel-body">
              <div className="eyebrow">LEDGER DIFF</div>
              <div className="mono mt-8" style={{ fontSize: 22 }}>
                0.00
              </div>
              <div className="hint mt-8">最近对账 16:41:24 · 通过</div>
            </div>
          </section>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>本机进程</h2>
            <div className="panel-meta">PowerShell 启动脚本 · 无容器依赖</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="svc-list">
              {d.services.map((s) => (
                <div className="svc" key={s.name}>
                  <div className="svc-name">{s.name}</div>
                  <div className="svc-role">
                    {s.role}
                    <span className="svc-hide"> · {s.note}</span>
                  </div>
                  <div className="svc-metric">{s.uptime}</div>
                  <div className="svc-metric svc-hide">P95 {s.p95}</div>
                  <div style={{ textAlign: "right" }}>
                    <Chip variant={s.state === "UP" ? "ok" : "warn"} dot>
                      {s.state}
                    </Chip>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>链准入</h2>
            <div className="panel-meta">6 阶段准入 · 当前最高 PAPER_READY</div>
          </header>
          <div className="panel-body">
            <div className="grid g-4">
              {d.chains.map((c) => {
                const cur = c.stage - 1;
                return (
                  <div className="chain-card" key={c.id}>
                    <div className="chain-top">
                      <div className="chain-name">
                        {c.name} <span>#{c.id}</span>
                      </div>
                      <Chip variant={c.stage >= 4 ? "ok" : c.stage >= 3 ? "warn" : "idle"}>{c.status}</Chip>
                    </div>
                    <div className="stage-ladder">
                      {d.CHAIN_STAGES.map((s, i) => (
                        <i
                          key={s}
                          className={i < cur ? "is-done" : i === cur ? "is-current" : ""}
                          title={s}
                        />
                      ))}
                    </div>
                    <div className="chain-kv">
                      <span>
                        原生 <b>{c.native}</b>
                      </span>
                      <span>
                        出块 <b>{c.blockTime}</b>
                      </span>
                      <span>
                        最终性 <b>{c.finality}</b>
                      </span>
                      <span>
                        Gas 模型 <b>{c.gasModel}</b>
                      </span>
                      <span>
                        RPC <b>{c.rpc}</b>
                      </span>
                      <span>
                        备 <b>{c.rpc2}</b>
                      </span>
                      <span>
                        高度 <b>{fmt.compact(c.block)}</b>
                      </span>
                      <span>
                        落后 <b>{c.lag}</b>
                      </span>
                      <span>
                        Gas <b>{c.gas}</b>
                      </span>
                      <span>
                        池 <b>{c.pools}</b>
                      </span>
                    </div>
                    <div className="chain-foot">
                      <span>{c.adapter}</span>
                      <span>{c.enabledAt}</span>
                      <span>{c.archive ? "ARCHIVE" : "NO ARCHIVE"}</span>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
          <div className="panel-note">
            某条链通过审计不代表其他链的部署地址与协议版本同样安全；每条链独立配置、独立验证。
          </div>
        </section>

        <div className="grid g-even">
          <section className="panel">
            <header className="panel-head">
              <h2>交易所适配</h2>
              <div className="panel-meta">仅公共行情</div>
            </header>
            <div className="panel-body panel-body--flush">
              <div className="tbl-wrap">
                <table className="tbl tbl-compact">
                  <thead>
                    <tr>
                      <th>交易所</th>
                      <th>适配器</th>
                      <th className="is-num">盘口年龄</th>
                      <th className="is-num">缺口</th>
                      <th>状态</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.exchanges.map((x) => (
                      <tr key={x.name}>
                        <td className="cell-strong">{x.name}</td>
                        <td className="mono muted" style={{ fontSize: 10 }}>
                          {x.adapter}
                        </td>
                        <td className="is-num">{x.bookAge}</td>
                        <td className={`is-num ${x.gaps ? "neg" : "muted"}`}>{x.gaps}</td>
                        <td>
                          <Chip
                            variant={
                              x.state === "ENABLED" ? "ok" : x.state === "INTERFACE_ONLY" ? "warn" : "idle"
                            }
                          >
                            {x.state}
                          </Chip>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
            <div className="panel-note">
              第一阶段不要求交易权限 API Key，且不允许提款权限；Paper 适配器无法调用 Live 适配器。
            </div>
          </section>

          <section className="panel">
            <header className="panel-head">
              <h2>运行基线</h2>
            </header>
            <div className="panel-body">
              <dl className="kv">
                <dt>数据库</dt>
                <dd>PostgreSQL · arbx_local</dd>
                <dt>原始数据</dt>
                <dd>Parquet 分区 · 4 链</dd>
                <dt>配置版本</dt>
                <dd>cfg-2026.11.19-b4</dd>
                <dt>风控版本</dt>
                <dd>v1.0.4</dd>
                <dt>链上模拟</dt>
                <dd>REVM · Fork 本地</dd>
                <dt>实盘签名器</dt>
                <dd className="neg">未接入</dd>
              </dl>
            </div>
          </section>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>DEX 协议与适配器</h2>
            <div className="panel-meta">V4 / Hook 默认阻断</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>协议</th>
                    <th>类型</th>
                    <th className="is-num">池</th>
                    <th className="is-num">链</th>
                    <th>适配器</th>
                    <th>状态</th>
                  </tr>
                </thead>
                <tbody>
                  {d.dexProtocols.map((p) => (
                    <tr key={p.name}>
                      <td className="cell-strong">{p.name}</td>
                      <td className="muted">{p.type}</td>
                      <td className="is-num">{p.pools || "—"}</td>
                      <td className="is-num">{p.chains || "—"}</td>
                      <td className="mono muted" style={{ fontSize: 10 }}>
                        {p.adapter}
                      </td>
                      <td>
                        <Chip variant={p.state === "ENABLED" ? "ok" : "bad"}>{p.state}</Chip>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">
            新池先进入观察与模拟，不直接进入执行白名单；代码或版本变化会立即暂停相关适配器。
          </div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>本机重启恢复检查</h2>
            <div className="panel-meta">TDD §15 · 顺序执行，全部通过后才恢复扫描</div>
          </header>
          <div className="panel-body">
            <div className="restart-list">
              {RESTART_STEPS.map(([title, note, tone], i) => (
                <div className="restart-item" key={title}>
                  <span className="restart-num">{i + 1}</span>
                  <div>
                    <div className="restart-title">{title}</div>
                    <div className="restart-note">{note}</div>
                  </div>
                  <Chip variant={tone === "ok" ? "ok" : "warn"}>{tone === "ok" ? "完成" : "部分"}</Chip>
                </div>
              ))}
            </div>
          </div>
          <div className="panel-note">
            本机休眠、断网、系统更新或重启后，必须停止 Paper 订单状态推进并将行情标记为断档。
          </div>
        </section>
      </div>
    </>
  );
}
