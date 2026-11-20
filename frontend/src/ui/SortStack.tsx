import { useEffect, useRef, useState } from "react";
import { Icon } from "../lib/icons";

export interface SortLevel<K extends string> {
  key: K;
  dir: -1 | 1;
}

export interface SortOption<K extends string> {
  key: K;
  label: string;
}

/**
 * 多条件排序控件：按顺序比较各级字段，前一级相等才看下一级。
 * - 点标签切换该级升降序
 * - 点 ✕ 移除该级
 * - 「+ 条件」从剩余字段里追加（最多 max 级）
 */
export function SortStack<K extends string>({
  levels,
  options,
  onChange,
  max = 3,
}: {
  levels: SortLevel<K>[];
  options: SortOption<K>[];
  onChange: (next: SortLevel<K>[]) => void;
  max?: number;
}) {
  const [open, setOpen] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (boxRef.current && !boxRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const label = (k: K) => options.find((o) => o.key === k)?.label ?? k;
  const used = new Set(levels.map((l) => l.key));
  const remaining = options.filter((o) => !used.has(o.key));

  const toggleDir = (i: number) => {
    const next = levels.map((l, idx) => (idx === i ? { ...l, dir: (l.dir === 1 ? -1 : 1) as -1 | 1 } : l));
    onChange(next);
  };
  const remove = (i: number) => onChange(levels.filter((_, idx) => idx !== i));
  const add = (k: K) => {
    onChange([...levels, { key: k, dir: -1 }]);
    setOpen(false);
  };

  return (
    <div className="sortstack" ref={boxRef}>
      <span className="eyebrow">SORT</span>
      {levels.map((lv, i) => (
        <span className="sortchip" key={lv.key}>
          {i > 0 && <em className="sortthen">then</em>}
          <button className="sortchip-main" onClick={() => toggleDir(i)} title="点击切换升/降序">
            {label(lv.key)}
            <b>{lv.dir === -1 ? "↓" : "↑"}</b>
          </button>
          <button className="sortchip-x" onClick={() => remove(i)} title="移除该排序条件" aria-label={`移除 ${label(lv.key)}`}>
            <Icon name="x" size={9} />
          </button>
        </span>
      ))}
      {levels.length < max && remaining.length > 0 && (
        <button className="sortadd" onClick={() => setOpen((v) => !v)} title="追加排序条件">
          + 条件
        </button>
      )}
      {open && (
        <div className="sortmenu" role="menu">
          {remaining.map((o) => (
            <button key={o.key} role="menuitem" onClick={() => add(o.key)}>
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
