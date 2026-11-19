import { Icon } from "../lib/icons";
import type { IconName } from "../lib/icons";
import { Chip } from "../ui/Chip";

type CapStatus = "ready" | "partial" | "off";

interface Cap {
  n: string;
  s: CapStatus;
  where: string;
  doc: string;
  note: string;
}

const CAPS: Cap[] = [
  { n: "多链扫描", s: "ready", where: "#/system", doc: "PRD §5.1", note: "4 条链注册，3 条 PAPER_READY；链级准入阶梯与双 RPC 一致性校验" },
  { n: "多 DEX / 多协议池监控", s: "ready", where: "#/pools", doc: "PRD §5.2", note: "7 协议 · 326 池 · 池 / Token / Adapter 三层白名单" },
  { n: "原子三角套利", s: "ready", where: "#/scanner-dex", doc: "PRD §2.2", note: "同链三跳闭环，逐跳最低输出约束，整笔回滚" },
  { n: "原子跨池套利", s: "ready", where: "#/scanner-dex", doc: "PRD §2.3", note: "跨 DEX / 跨池价差，V3 / V2 / StableSwap / Weighted 分类适配" },
  { n: "CEX–DEX 链外对冲", s: "ready", where: "#/scanner-cex", doc: "PRD §2.4", note: "非原子双腿，部分成交 / 回滚 / 状态未知全部分支" },
  { n: "多路径搜索与最优金额", s: "partial", where: "#/scanner-dex", doc: "TDD §4.2", note: "已表达搜索规模（候选→剪枝→模拟）与最优名义容量曲线；逐条候选路径尚未单独视图" },
  { n: "完整盈亏推演", s: "ready", where: "机会抽屉", doc: "PRD §5.5", note: "M0–M3 四模型 + 成本桥 + 容量曲线 + 三类失效模型与最坏情景" },
  { n: "模拟实盘交易", s: "ready", where: "#/executions", doc: "PRD §5.6", note: "8 条执行轨迹，覆盖对冲成功 / 部分成交 / 回滚 / 状态未知 / UNWIND / 人工处置" },
  { n: "真实实盘交易", s: "off", where: "#/contract", doc: "PRD §6 / §4", note: "第一阶段明确不进行真实交易；界面为关闭态，需独立验收后开启" },
  { n: "风控（资金 / 交易 / 合约 / 系统）", s: "ready", where: "#/risk", doc: "风控 §3–§10", note: "44 项参数 · 6 组 · 10 条熔断规则 · 五级范围开关 · 合约执行边界" },
  { n: "交易记录与收益分析", s: "ready", where: "#/ledger · #/performance", doc: "TDD §9 / §15", note: "10 类账户、策略账本、闪电贷流水、权益与回撤、15 项运行指标" },
];

const STATUS: Record<CapStatus, { variant: "ok" | "warn" | "bad"; label: string }> = {
  ready: { variant: "ok", label: "已具备" },
  partial: { variant: "warn", label: "部分具备" },
  off: { variant: "bad", label: "未启用" },
};

interface Mode {
  id: string;
  name: string;
  paper: string;
  live: boolean;
  note: string;
}

const MODES: Mode[] = [
  { id: "dex_triple", name: "策略 1 · 原子三角套利", paper: "运行中", live: false, note: "三跳闭环，Flash Loan / 自有资金" },
  { id: "dex_cross", name: "策略 2 · 原子跨池套利", paper: "运行中", live: false, note: "跨协议价差，逐池校验" },
  { id: "cex_dex", name: "策略 3 · CEX–DEX 链外对冲", paper: "运行中", live: false, note: "双腿对冲，含补偿与残余敞口处置" },
];

/** [条件, 当前状态, 是否已满足] */
const LIVE_GATES: [string, string, boolean][] = [
  ["合约测试全部通过", "当前 37 / 39，资金安全 2 项未过", false],
  ["独立安全审计", "未开始", false],
  ["实盘签名器独立部署", "未接入，Paper 无法调用", false],
  ["CEX 交易 API 与提款权限分离", "未申请交易权限", false],
  ["独立实盘账本与额度白名单", "未建立", false],
  ["小额候选上限 250 USDC 独立验收", "未执行", false],
];

export function Capability() {
  const ready = CAPS.filter((c) => c.s === "ready").length;
  const partial = CAPS.filter((c) => c.s === "partial").length;
  const off = CAPS.filter((c) => c.s === "off").length;

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">14</span> GOVERN · CAPABILITY
          </div>
          <h1>能力矩阵</h1>
          <p className="page-sub">
            逐项核对系统能力，标注「已具备 / 部分 / 未启用」以及对应界面与文档依据。实盘属于范围决策，不是缺失。
          </p>
        </div>
        <div className="page-actions">
          <span className="chip chip--ok">{ready} 已具备</span>
          <span className="chip chip--warn">{partial} 部分</span>
          <span className="chip chip--bad">{off} 未启用</span>
        </div>
      </div>

      <div className="stack">
        <section className="panel">
          <header className="panel-head">
            <h2>
              能力清单 <span className="count">{CAPS.length}</span>
            </h2>
            <div className="panel-meta">点击「查看」跳到对应界面</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th style={{ width: 34 }} />
                    <th>能力</th>
                    <th>状态</th>
                    <th>依据</th>
                    <th>说明</th>
                    <th style={{ width: 70 }} />
                  </tr>
                </thead>
                <tbody>
                  {CAPS.map((c, i) => {
                    const st = STATUS[c.s];
                    return (
                      <tr key={c.n}>
                        <td className="is-num muted" style={{ fontSize: 9.5 }}>
                          {String(i + 1).padStart(2, "0")}
                        </td>
                        <td className="cell-strong">{c.n}</td>
                        <td>
                          <Chip variant={st.variant} dot>
                            {st.label}
                          </Chip>
                        </td>
                        <td>
                          <span className="tag-mini">{c.doc}</span>
                        </td>
                        <td className="muted" style={{ whiteSpace: "normal", fontSize: 10.5, lineHeight: 1.5 }}>
                          {c.note}
                        </td>
                        <td>
                          <a
                            className="btn btn--sm btn--ghost"
                            href={c.where.startsWith("#/") ? c.where : "#/scanner-dex"}
                          >
                            查看 <Icon name="chevron" size={12} />
                          </a>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">
            「已具备」指前端已完整表达该能力的对象、状态与失败分支；后端实现不在本原型范围内。
          </div>
        </section>

        <section className="panel">
          <header className="panel-head">
            <h2>策略 × 运行模式</h2>
            <div className="panel-meta">PAPER 全部运行中 · LIVE 全部关闭</div>
          </header>
          <div className="panel-body panel-body--flush">
            <div className="tbl-wrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>策略</th>
                    <th>说明</th>
                    <th>Paper 模拟</th>
                    <th>真实实盘</th>
                    <th>策略配置</th>
                  </tr>
                </thead>
                <tbody>
                  {MODES.map((m) => (
                    <tr key={m.id}>
                      <td className="cell-strong">{m.name}</td>
                      <td className="muted" style={{ fontSize: 10.5 }}>
                        {m.note}
                      </td>
                      <td>
                        <Chip variant="ok" dot>
                          {m.paper}
                        </Chip>
                      </td>
                      <td>
                        <Chip variant="bad" dot>
                          未启用
                        </Chip>
                      </td>
                      <td>
                        <a className="btn btn--sm btn--ghost" href={`#/strategy/${m.id}`}>
                          打开 <Icon name="chevron" size={12} />
                        </a>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
          <div className="panel-note">
            三个策略共用同一套策略对象、风险检查与交易计划格式；Paper 与 Live 只替换 Adapter、账户与账本，不复制策略逻辑。
          </div>
        </section>

        <div className="grid g-main">
          <section className="panel">
            <header className="panel-head">
              <h2>实盘启用前置条件</h2>
              <div className="panel-meta">全部满足后才允许进入小额实盘验证</div>
            </header>
            <div className="panel-body">
              <div className="check-list">
                {LIVE_GATES.map(([name, note, ok]) => {
                  const icon: IconName = ok ? "check" : "alert";
                  return (
                    <div className="check-item" key={name} style={ok ? undefined : { color: "var(--ink-3)" }}>
                      <Icon name={icon} size={11} />
                      <span>
                        {name}
                        <span style={{ display: "block", fontSize: 9.5, color: "var(--ink-4)" }}>{note}</span>
                      </span>
                    </div>
                  );
                })}
              </div>
            </div>
            <div className="panel-note">
              第一阶段文档明确「不进行主网真实交易、不提交真实 CEX 订单」；开启实盘属于范围变更，需单独确认与验收。
            </div>
          </section>

          <section className="panel">
            <header className="panel-head">
              <h2>核对结论</h2>
              <div className="panel-meta">11 项</div>
            </header>
            <div className="panel-body">
              <dl className="kv">
                <dt>已具备</dt>
                <dd className="pos">{ready} 项</dd>
                <dt>部分具备</dt>
                <dd className="warn">{partial} 项</dd>
                <dt>未启用</dt>
                <dd className="neg">{off} 项</dd>
                <dt>Paper 可跑策略</dt>
                <dd>3 / 3</dd>
                <dt>Live 可跑策略</dt>
                <dd className="neg">0 / 3</dd>
              </dl>
              <div className="divider" />
              <p className="hint">
                唯一缺口是「真实实盘交易」：它不是没设计，而是被文档限定在第一阶段之外。前端已按「可开启但默认关闭」的方式预留了状态位。
              </p>
            </div>
          </section>
        </div>
      </div>
    </>
  );
}
