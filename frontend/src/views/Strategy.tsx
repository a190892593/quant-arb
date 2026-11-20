import { useState } from "react";
import { data } from "../data";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { Chip, StatusChip } from "../ui/Chip";
import { toast } from "../ui/Toast";

/* ------------------------------------------------------------------ types */

interface CfgItem {
  key: string;
  label: string;
  value: number;
  unit: string;
  min: number;
  max: number;
  step: number;
  global: number;
}

interface CfgGroup {
  name: string;
  note?: string;
  items: CfgItem[];
}

interface CfgToggle {
  key: string;
  label: string;
  on: boolean;
  note: string;
  locked: boolean;
}

interface CfgChange {
  at: string;
  by: string;
  change: string;
}

interface CfgKpi {
  label: string;
  value: string;
  note: string;
  tone?: "pos" | "neg" | "warn";
}

interface StrategyConfig {
  id: string;
  name: string;
  family: string;
  thesis: string;
  mode: string;
  enabled: boolean;
  version: string;
  kpi: CfgKpi[];
  groups: CfgGroup[];
  toggles: CfgToggle[];
  history: CfgChange[];
}

/** editable working copy of a strategy config */
interface Draft {
  enabled: boolean;
  groups: CfgGroup[];
  toggles: CfgToggle[];
  history: CfgChange[];
}

interface RowError {
  bad: boolean;
  msg: string;
}

const CONFIG = (data as unknown as { strategyConfig: Record<string, StrategyConfig> }).strategyConfig;

/* params where a larger value is the stricter choice */
const TIGHTER_MIN: string[] = ["minNet", "cooldown", "minOutBuffer", "minTvl", "depthMultiple", "delayReserve", "competition"];

const ORDER: Record<string, string> = { dex_triple: "S1", dex_cross: "S2", cex_dex: "S3" };

const FOOT: string[] = [
  "覆盖规则不得突破全局硬限制：只能收紧，不能放宽。",
  "单笔名义上限仍受池深度、库存与全局额度约束。",
  "超过报价年龄或区块差的机会直接失效，不进入执行计划。",
  "参数变更需记录原因与生效时间，并写入版本。",
];

/* ---------------------------------------------------------------- helpers */

const cloneGroups = (groups: CfgGroup[]): CfgGroup[] =>
  groups.map((g) => ({ ...g, items: g.items.map((i) => ({ ...i })) }));

const cloneToggles = (toggles: CfgToggle[]): CfgToggle[] => toggles.map((t) => ({ ...t }));

const cloneHistory = (history: CfgChange[]): CfgChange[] => history.map((h) => ({ ...h }));

const draftFrom = (cfg: StrategyConfig): Draft => ({
  enabled: cfg.enabled,
  groups: cloneGroups(cfg.groups),
  toggles: cloneToggles(cfg.toggles),
  history: cloneHistory(cfg.history),
});

/** a strategy override may only tighten the global limit, never loosen it */
const looser = (item: CfgItem, v: number): boolean => {
  if (!item.global) return false;
  return TIGHTER_MIN.includes(item.key) ? v < item.global : v > item.global;
};

const rowError = (item: CfgItem, text: string): RowError => {
  const v = Number(text);
  if (!Number.isFinite(v)) return { bad: true, msg: `超出允许范围 ${item.min} – ${item.max}` };
  if (looser(item, v))
    return { bad: true, msg: "不得放宽全局限制（全局 " + item.global + " " + item.unit + "）" };
  if (v < item.min || v > item.max) return { bad: true, msg: `超出允许范围 ${item.min} – ${item.max}` };
  return { bad: false, msg: "" };
};

const kpiTone = (tone: CfgKpi["tone"]): string =>
  tone === "pos" ? "pos" : tone === "neg" ? "neg" : tone === "warn" ? "warn" : "";

const pad2 = (n: number): string => String(n).padStart(2, "0");

/* -------------------------------------------------------------- component */

export function Strategy() {
  const id = location.hash.split("/")[2] || "dex_triple";
  const cfg = CONFIG[id] ?? CONFIG.dex_triple;

  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [dirtyMap, setDirtyMap] = useState<Record<string, boolean>>({});
  /* raw text as typed, so an invalid keystroke stays visible while editing */
  const [raw, setRaw] = useState<Record<string, string>>({});

  const draft = drafts[id] ?? draftFrom(cfg);
  const dirty = dirtyMap[id] === true;

  const textOf = (gi: number, ii: number, item: CfgItem): string =>
    raw[`${id}:${gi}-${ii}`] ?? String(item.value);

  const errors: RowError[][] = draft.groups.map((g, gi) =>
    g.items.map((item, ii) => rowError(item, textOf(gi, ii, item))),
  );

  const update = (fn: (d: Draft) => Draft) => {
    setDrafts((prev) => ({ ...prev, [id]: fn(prev[id] ?? draftFrom(cfg)) }));
    setDirtyMap((prev) => (prev[id] ? prev : { ...prev, [id]: true }));
  };

  const onParam = (gi: number, ii: number, text: string) => {
    const item = draft.groups[gi].items[ii];
    setRaw((prev) => ({ ...prev, [`${id}:${gi}-${ii}`]: text }));
    if (rowError(item, text).bad) return;
    const v = Number(text);
    update((d) => ({
      ...d,
      groups: d.groups.map((g, x) =>
        x === gi ? { ...g, items: g.items.map((it, y) => (y === ii ? { ...it, value: v } : it)) } : g,
      ),
    }));
  };

  const onToggle = (ti: number) => {
    if (draft.toggles[ti].locked) return;
    update((d) => ({
      ...d,
      toggles: d.toggles.map((t, i) => (i === ti ? { ...t, on: !t.on } : t)),
    }));
  };

  /** re-render parity: the prototype rebuilds the markup, so raw keystrokes are dropped */
  const resetRaw = () =>
    setRaw((prev) => Object.fromEntries(Object.entries(prev).filter(([k]) => !k.startsWith(id + ":"))));

  const onEnabled = () => {
    const next = !draft.enabled;
    setDrafts((prev) => ({ ...prev, [id]: { ...(prev[id] ?? draft), enabled: next } }));
    cfg.enabled = next;
    resetRaw();
    toast(next ? "策略已启用" : "策略已停用", {
      sub: next ? "重新参与扫描，仍受全局硬风控约束" : "不再生成新机会，已有持仓与订单不受影响",
      variant: next ? undefined : "warn",
    });
  };

  const onSave = () => {
    if (errors.some((g) => g.some((e) => e.bad))) {
      toast("存在不合法的参数", { sub: "请修正红色标记的字段后再保存", variant: "bad" });
      return;
    }
    cfg.groups = cloneGroups(draft.groups);
    cfg.toggles = cloneToggles(draft.toggles);
    const stamp = new Date();
    cfg.version = "cfg-2026.11.19-b" + (5 + Math.floor(Math.random() * 3));
    const entry: CfgChange = {
      at: "11-19 " + pad2(stamp.getHours()) + ":" + pad2(stamp.getMinutes()),
      by: "operator",
      change: "策略参数更新 · " + cfg.version,
    };
    cfg.history.unshift(entry);
    setDrafts((prev) => ({
      ...prev,
      [id]: {
        enabled: draft.enabled,
        groups: cloneGroups(draft.groups),
        toggles: cloneToggles(draft.toggles),
        history: [entry, ...cloneHistory(draft.history)],
      },
    }));
    setDirtyMap((prev) => ({ ...prev, [id]: false }));
    resetRaw();
    toast("策略参数已保存", { sub: cfg.name + " · " + cfg.version });
  };

  const recentOpps = data.opps.filter((o) => o.strategy === id).slice(0, 3);

  return (
    <>
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">
            <span className="idx">{ORDER[id] || "S"}</span> STRATEGY · {cfg.family}
          </div>
          <h1>{cfg.name}</h1>
          <p className="page-sub">{cfg.thesis}</p>
        </div>
        <div className="page-actions">
          <Chip variant="warn">PAPER</Chip>
          <Chip variant={draft.enabled ? "ok" : "idle"} dot>
            {draft.enabled ? "ENABLED" : "DISABLED"}
          </Chip>
          {dirty && <Chip variant="accent">未保存</Chip>}
          <button className="btn" onClick={onEnabled}>
            <Icon name={draft.enabled ? "pause" : "play"} /> {draft.enabled ? "停用策略" : "启用策略"}
          </button>
          <button className="btn btn--primary" onClick={onSave} disabled={!dirty}>
            <Icon name="check" /> 保存修改
          </button>
        </div>
      </div>

      <div className="stack">
        <div className="kpi-grid">
          {cfg.kpi.map((k) => (
            <div className="kpi" key={k.label}>
              <div className="kpi-label">
                {k.label}
                <span className="unit">TODAY</span>
              </div>
              <div className={`kpi-value ${kpiTone(k.tone)}`}>{k.value}</div>
              <div className="kpi-foot">
                <span>{k.note}</span>
              </div>
            </div>
          ))}
        </div>

        <div className="grid g-main">
          <div className="stack">
            {draft.groups.map((g, gi) => (
              <section className="panel" key={`${gi}-${g.name}`}>
                <header className="panel-head">
                  <h2>{g.name}</h2>
                  <div className="panel-meta">{g.note ? g.note : "策略级覆盖"}</div>
                </header>
                <div className="panel-body">
                  <div className="cfg-list">
                    {g.items.map((item, ii) => {
                      const err = errors[gi][ii];
                      return (
                        <div
                          className={`cfg-row${err.bad ? " is-invalid" : ""}`}
                          data-row={`${gi}-${ii}`}
                          data-error={err.bad ? err.msg : undefined}
                          key={item.key}
                        >
                          <div className="cfg-label">
                            {item.label}
                            <span>
                              全局 {fmt.num(item.global, item.global % 1 ? 1 : 0)} {item.unit}
                            </span>
                          </div>
                          <div className="cfg-field">
                            <input
                              type="number"
                              data-g={gi}
                              data-i={ii}
                              value={textOf(gi, ii, item)}
                              min={item.min}
                              max={item.max}
                              step={item.step}
                              aria-label={item.label}
                              onChange={(e) => onParam(gi, ii, e.target.value)}
                            />
                            <span className="cfg-unit">{item.unit}</span>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
                <div className="panel-note">{FOOT[gi] || "参数变更需记录原因与生效时间。"}</div>
              </section>
            ))}

            <div className="note">
              <b>策略参数只影响本策略的机会筛选与计划生成。</b>{" "}
              硬风控（日亏损、最大回撤、敞口、盘口完整性）在后端强制执行，无法通过本页关闭。
            </div>
          </div>

          <div className="stack">
            <section className="panel">
              <header className="panel-head">
                <h2>执行开关</h2>
                <div className="panel-meta">
                  {cfg.toggles.filter((t) => t.locked).length} 项硬性要求
                </div>
              </header>
              <div className="panel-body">
                <div className="cfg-toggles">
                  {draft.toggles.map((t, ti) => (
                    <div className={`cfg-toggle${t.locked ? " is-locked" : ""}`} key={t.key}>
                      <div>
                        <div className="cfg-toggle-label">
                          {t.label}
                          {t.locked && (
                            <>
                              {" "}
                              <span className="tag-mini">LOCKED</span>
                            </>
                          )}
                        </div>
                        {t.note ? <div className="cfg-toggle-note">{t.note}</div> : null}
                      </div>
                      <button
                        className="toggle"
                        role="switch"
                        aria-pressed={t.on}
                        disabled={t.locked}
                        data-toggle={ti}
                        aria-label={t.label}
                        onClick={() => onToggle(ti)}
                      />
                    </div>
                  ))}
                </div>
              </div>
              <div className="panel-note">锁定项由安全规格强制，任何策略配置都不能关闭。</div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>作用域与继承</h2>
                <div className="panel-meta">全局 → 策略 → 交易对</div>
              </header>
              <div className="panel-body">
                <div className="cfg-scope">
                  <div>
                    <span>全局默认</span>
                    <b>{cfg.version}</b>
                  </div>
                  <div>
                    <span>策略覆盖</span>
                    <b>本页</b>
                  </div>
                  <div>
                    <span>交易对覆盖</span>
                    <b>未设置</b>
                  </div>
                </div>
                <div className="divider" />
                <p className="hint">例如全局最大日亏损为 1%，本策略可收紧到 0.5%，但不能放宽到 2%。</p>
              </div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>变更记录</h2>
                <div className="panel-meta">最近 3 条</div>
              </header>
              <div className="panel-body panel-body--flush">
                <div className="ev-list">
                  {draft.history.map((h) => (
                    <div className="ev" key={h.at + h.change}>
                      <div className="ev-top">
                        <span className="ev-scope">{h.change}</span>
                        <span className="ev-time">{h.at}</span>
                      </div>
                      <div className="ev-msg">操作者 {h.by}</div>
                    </div>
                  ))}
                </div>
              </div>
            </section>

            <section className="panel">
              <header className="panel-head">
                <h2>最近机会</h2>
                <div className="panel-meta">
                  <a
                    className="btn btn--sm btn--ghost"
                    href="#/scanner-dex"
                  >
                    查看扫描 <Icon name="chevron" />
                  </a>
                </div>
              </header>
              <div className="panel-body panel-body--flush">
                <div className="ev-list">
                  {recentOpps.map((o) => (
                    <div className="ev" key={o.id}>
                      <div className="ev-top">
                        <span className="ev-scope mono" style={{ fontSize: 10.5 }}>
                          {o.id}
                        </span>
                        <StatusChip status={o.status} />
                        <span className={`ev-time ${o.net >= 0 ? "pos" : "neg"}`}>{fmt.signed(o.net)}</span>
                      </div>
                      <div className="ev-msg">
                        {o.symbol || o.pair} · {o.venue}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
              <div className="panel-note">机会按最新检出排序；点击进入扫描页查看完整推演。</div>
            </section>
          </div>
        </div>
      </div>
    </>
  );
}
