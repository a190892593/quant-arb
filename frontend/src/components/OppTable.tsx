import { useMemo, useState, type ReactNode } from "react";
import type { Opportunity } from "../types";
import { data } from "../data";
import { fmt, signClass } from "../lib/format";
import { Icon } from "../lib/icons";
import { QualityChip, StatusChip } from "../ui/Chip";

type SortKey = "net" | "notional" | "stress" | "risk" | "age";

const sortVal = (o: Opportunity, k: SortKey) =>
  k === "risk" ? o.riskScore : k === "notional" ? o.notional : k === "age" ? o.ageMs : k === "stress" ? o.stressNet : o.net;

export interface OppTableProps {
  rows: Opportunity[];
  variant: "dex" | "cex";
  title?: string;
  meta?: ReactNode;
  showStrategyFilter?: boolean;
  note: string;
  emptyText: string;
  onOpen: (id: string) => void;
}

export function OppTable({
  rows,
  variant,
  title = variant === "dex" ? "原子机会流" : "双腿机会流",
  meta,
  showStrategyFilter = false,
  note,
  emptyText,
  onOpen,
}: OppTableProps) {
  const [strategy, setStrategy] = useState<"ALL" | "dex_triple" | "dex_cross">("ALL");
  const [status, setStatus] = useState("ALL");
  const [q, setQ] = useState("");
  const [sort, setSort] = useState<SortKey>("net");
  const [dir, setDir] = useState<-1 | 1>(-1);

  const list = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return rows
      .filter((o) => (showStrategyFilter && strategy !== "ALL" ? o.strategy === strategy : true))
      .filter((o) => (status === "ALL" ? true : o.status === status))
      .filter((o) =>
        needle
          ? `${o.id} ${o.symbol ?? ""} ${o.pair} ${o.venue}`.toLowerCase().includes(needle)
          : true,
      )
      .sort((a, b) => (sortVal(a, sort) - sortVal(b, sort)) * dir);
  }, [rows, strategy, status, q, sort, dir, showStrategyFilter]);

  const act = list.filter((o) => o.status === "ACTIONABLE");
  const freshest = [...list].sort((a, b) => a.ageMs - b.ageMs)[0];

  const onSort = (k: SortKey) => {
    if (sort === k) setDir((d) => (d === 1 ? -1 : 1));
    else {
      setSort(k);
      setDir(-1);
    }
  };

  const arrow = (k: SortKey) => (sort === k ? (dir === 1 ? "↑" : "↓") : "");

  const segBtn = (active: boolean) => (active ? "is-active" : "");

  return (
    <section className="panel">
      <header className="panel-head">
        <h2>
          {title} <span className="count">{list.length} / {rows.length}</span>
        </h2>
        <div className="panel-meta">{meta ?? "净利已扣除全部执行成本"}</div>
      </header>

      <div className="toolbar">
        <div className="filters">
          {showStrategyFilter && (
            <div className="seg" role="group" aria-label="策略筛选">
              {([["ALL", "全部原子"], ["dex_triple", "三角"], ["dex_cross", "跨池"]] as const).map(([v, label]) => (
                <button key={v} className={segBtn(strategy === v)} onClick={() => setStrategy(v)}>
                  {label}
                </button>
              ))}
            </div>
          )}
          <div className="seg" role="group" aria-label="状态筛选">
            {(["ALL", "ACTIONABLE", "WATCH", "REJECTED"] as const).map((v) => (
              <button key={v} className={segBtn(status === v)} onClick={() => setStatus(v)}>
                {v === "ALL" ? "全部" : v === "ACTIONABLE" ? "可执行" : v === "WATCH" ? "观察" : "拒绝"}
              </button>
            ))}
          </div>
          <label className="search">
            <Icon name="search" size={13} />
            <input
              type="search"
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder={variant === "cex" ? "ID / 交易对 / 场所" : "ID / 路线 / 协议"}
              aria-label="搜索机会"
            />
          </label>
        </div>
        <span className="hint">点击行查看盈亏推演</span>
      </div>

      <div className="panel-body panel-body--flush">
        <div className="tbl-wrap">
          <table className="tbl opp-tbl">
            <thead>
              <tr>
                <th>机会</th>
                <th>策略</th>
                <th>{variant === "cex" ? "交易对 / 方向" : "场所 / 路线"}</th>
                <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("notional")}>名义 {arrow("notional")}</th>
                <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("net")}>净利 {arrow("net")}</th>
                <th className="is-num">bps</th>
                <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("stress")}>压力净利 {arrow("stress")}</th>
                <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("risk")}>风险 {arrow("risk")}</th>
                <th>数据</th>
                <th>状态</th>
                <th className="is-num" style={{ cursor: "pointer" }} onClick={() => onSort("age")}>年龄 {arrow("age")}</th>
              </tr>
            </thead>
            <tbody>
              {list.length === 0 && (
                <tr>
                  <td colSpan={11}>
                    <div className="empty">{emptyText}</div>
                  </td>
                </tr>
              )}
              {list.map((o) => (
                <tr
                  key={o.id}
                  className={`is-link${freshest?.id === o.id ? " is-fresh" : ""}`}
                  onClick={() => onOpen(o.id)}
                >
                  <td className="cell-id">{o.id}</td>
                  <td>
                    <span className="tag-mini">{data.STRATEGY[o.strategy].short}</span>
                  </td>
                  <td className="cell-strong">
                    {o.symbol ?? o.venue}
                    <span className="cell-sub">{o.pair}</span>
                  </td>
                  <td className="is-num">{fmt.num(o.notional, 0)}</td>
                  <td className={`is-num ${signClass(o.net)}`}>{fmt.signed(o.net)}</td>
                  <td className="is-num muted">{fmt.num(o.netBps, 1)}</td>
                  <td className={`is-num ${signClass(o.stressNet)}`}>{fmt.signed(o.stressNet)}</td>
                  <td className={`is-num ${o.riskScore >= 70 ? "pos" : o.riskScore >= 50 ? "" : "neg"}`}>{o.riskScore}</td>
                  <td>
                    <QualityChip quality={o.quality} />
                  </td>
                  <td>
                    <StatusChip status={o.status} />
                    {o.reasons && o.reasons.length > 0 && (
                      <div className="reasons">
                        {o.reasons.slice(0, 2).map((r) => (
                          <span key={r}>{r}</span>
                        ))}
                      </div>
                    )}
                  </td>
                  <td className="is-num muted">{fmt.age(o.ageMs)}</td>
                </tr>
              ))}
            </tbody>
            {act.length > 0 && (
              <tfoot>
                <tr>
                  <td colSpan={3} className="muted" style={{ fontSize: 10, letterSpacing: ".08em" }}>
                    可执行合计 · {act.length} 条
                  </td>
                  <td className="is-num muted">{fmt.num(act.reduce((s, o) => s + o.notional, 0), 0)}</td>
                  <td className="is-num pos">{fmt.signed(act.reduce((s, o) => s + o.net, 0))}</td>
                  <td colSpan={6} className="muted" style={{ fontSize: 9.5 }}>
                    名义受深度与库存约束，不等于可同时执行
                  </td>
                </tr>
              </tfoot>
            )}
          </table>
        </div>
      </div>
      <div className="panel-note">{note}</div>
    </section>
  );
}
