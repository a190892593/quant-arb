import { data } from "../data";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip } from "../ui/Chip";
import { toast } from "../ui/Toast";

/**
 * Contract artefacts (合约 §2 / §13) ship inside the legacy dataset but are not
 * part of DataModel in src/types.ts yet; keep the slice typed locally until the
 * API boundary owns it.
 */
interface ContractComponent {
  name: string;
  role: string;
  addr: string;
  version: string;
  state: string;
  note: string;
}

interface ContractTest {
  group: string;
  total: number;
  pass: number;
  note: string;
}

interface ContractEvent {
  name: string;
  n: number;
  note: string;
}

interface ContractModel {
  contractComponents: ContractComponent[];
  contractTests: ContractTest[];
  contractEvents: ContractEvent[];
}

const d = data as unknown as ContractModel;

const BOUNDARIES = [
  "不允许任意外部调用目标",
  "不允许任意 Delegatecall",
  "不允许未经批准的 Router / Pool",
  "不允许无期限授权",
  "不允许未经批准的 Token",
  "不允许开放任意提款地址",
  "每一跳必须有最低输出约束",
  "最终闭环必须回到起始资产",
];

const PAUSE_RULES = [
  "全局暂停已触发", "Chain ID 与计划不符", "Provider / Adapter 被禁用", "Token 被禁用",
  "交易计划已过期", "Nonce 已使用", "计划签名不合法", "执行状态不为空闲",
  "当前 Gas Price 超限", "路线或利润条件不满足",
];

export function Contract() {
  const totalTests = d.contractTests.reduce((s, t) => s + t.total, 0);
  const passed = d.contractTests.reduce((s, t) => s + t.pass, 0);
  const indexedEvents = d.contractEvents.reduce((s, e) => s + e.n, 0);

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow"><span className="idx">10</span> GOVERN · CONTRACT</div>
          <h1>执行合约</h1>
          <p className="page-sub">第一阶段只做本地编译、EVM / Fork 测试与接口记录。未完成测试与独立审计前，不部署主网、不启用真实签名。</p>
        </div>
        <div className="page-actions">
          <span className="chip chip--warn">LOCAL ONLY</span>
          <button
            className="btn"
            onClick={() =>
              toast("合约测试已启动", {
                sub: `${passed} / ${totalTests} 通过 · 资金安全 2 项失败，详见测试报告`,
                variant: "warn",
              })
            }
          >
            <Icon name="play" /> 运行合约测试
          </button>
          <button
            className="btn"
            onClick={() =>
              toast("字节码记录已导出", {
                sub: `${d.contractComponents.length} 个组件 · 含代码哈希与版本号`,
              })
            }
          >
            <Icon name="external" /> 导出字节码记录
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          <div className="kpi">
            <div className="kpi-label">合约组件<span className="unit">COMPONENTS</span></div>
            <div className="kpi-value">{d.contractComponents.length}</div>
            <div className="kpi-foot"><span>全部 v1.0.0-rc3 · 本地部署</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">测试通过<span className="unit">TESTS</span></div>
            <div className="kpi-value">{passed}<small>/ {totalTests}</small></div>
            <div className="kpi-foot"><span className="warn">资金安全 2 项未通过</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">链上事件<span className="unit">EVENTS</span></div>
            <div className="kpi-value">{d.contractEvents.length}</div>
            <div className="kpi-foot"><span>{fmt.num(indexedEvents, 0)} 条已索引</span></div>
          </div>
          <div className="kpi">
            <div className="kpi-label">主网部署<span className="unit">MAINNET</span></div>
            <div className="kpi-value neg" style={{ fontSize: 22, letterSpacing: 0 }}>未部署</div>
            <div className="kpi-foot"><span>无实盘签名器 · 无提款权限</span></div>
          </div>
        </div>

        <section className="panel">
          <header className="panel-head">
            <h2>合约组件 <span className="count">{d.contractComponents.length}</span></h2>
            <div className="panel-meta">角色权限分离 · 执行中不可提款</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead><tr><th>组件</th><th>职责</th><th>地址</th><th>版本</th><th>说明</th><th>状态</th></tr></thead>
                <tbody>
                  {d.contractComponents.map((c) => (
                    <tr key={c.name}>
                      <td className="cell-strong mono" style={{ fontSize: 11 }}>{c.name}</td>
                      <td className="muted">{c.role}</td>
                      <td className="cell-id">{c.addr}</td>
                      <td><span className="tag-mini">{c.version}</span></td>
                      <td className="muted" style={{ fontSize: 10.5 }}>{c.note}</td>
                      <td><Chip variant="warn" dot>{c.state === "LOCAL_ONLY" ? "LOCAL" : c.state}</Chip></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">闪电贷回调通过独立 Receiver Adapter 处理，不同协议不共用回调 ABI；回调必须校验调用方、Initiator、金额、费用与计划哈希。</div>
        </section>

        <div className="grid g-main">
          <section className="panel">
            <header className="panel-head">
              <h2>测试清单 <span className="count">{passed} / {totalTests}</span></h2>
              <div className="panel-meta">单元 · Fuzz · Invariant · Fork</div>
            </header>
            <div className="panel-body">
              <div className="stack" style={{ gap: 13 }}>
                {d.contractTests.map((t) => (
                  <div key={t.group}>
                    <div className="row between" style={{ marginBottom: 5 }}>
                      <span style={{ fontSize: 11.5, color: "var(--ink-1)" }}>{t.group}</span>
                      <span
                        className="mono"
                        style={{ fontSize: 10, color: t.pass === t.total ? "var(--pos)" : "var(--warn)" }}
                      >
                        {t.pass} / {t.total}
                      </span>
                    </div>
                    <div className={`bar ${t.pass === t.total ? "bar--ok" : "bar--warn"}`}>
                      <i style={{ width: `${(t.pass / t.total) * 100}%` }} />
                    </div>
                    <div className="hint" style={{ marginTop: 4 }}>{t.note}</div>
                  </div>
                ))}
              </div>
            </div>
            <div className="panel-note">资金安全项包含：偿还不足、最低利润不足、Token 转账失败、非标准 Token、授权不足、执行中提款。未全部通过前不得进入审计评估。</div>
          </section>

          <div className="stack">
            <section className="panel">
              <header className="panel-head"><h2>事件记录</h2><div className="panel-meta">链下已关联账本</div></header>
              <div className="panel-body panel-body--flush">
                <div className="ev-list">
                  {d.contractEvents.map((e) => (
                    <div className="ev" key={e.name}>
                      <div className="ev-top">
                        <span className="ev-scope mono" style={{ fontSize: 10.5 }}>{e.name}</span>
                        <span className="ev-time">{fmt.num(e.n, 0)}</span>
                      </div>
                      <div className="ev-msg">{e.note}</div>
                    </div>
                  ))}
                </div>
              </div>
            </section>
          </div>
        </div>

        <div className="grid g-even">
          <section className="panel">
            <header className="panel-head"><h2>执行边界</h2><div className="panel-meta">§8.5</div></header>
            <div className="panel-body">
              <div className="check-list">
                {BOUNDARIES.map((b) => (
                  <div className="check-item" key={b}><Icon name="check" size={11} /><span>{b}</span></div>
                ))}
              </div>
            </div>
          </section>

          <section className="panel">
            <header className="panel-head"><h2>暂停条件</h2><div className="panel-meta">§9.1 · 任一命中即拒绝新执行</div></header>
            <div className="panel-body">
              <div className="check-list">
                {PAUSE_RULES.map((b) => (
                  <div className="check-item" key={b}><Icon name="check" size={11} /><span>{b}</span></div>
                ))}
              </div>
            </div>
          </section>
        </div>
      </div>
    </>
  );
}
