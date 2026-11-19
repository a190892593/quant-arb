import { useEffect, useMemo, useRef, useState } from "react";
import { data } from "../data";
import { useDataStatus } from "../data";
import { fmt } from "../lib/format";
import { Icon } from "../lib/icons";
import { NAV } from "./nav";

export function CommandPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [idx, setIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const status = useDataStatus();

  const pool = useMemo(
    () => [
      ...NAV.flatMap((g) => g.items.map((i) => ({ label: i.label, hint: g.label.split(" ")[0], hash: i.hash }))),
      ...data.opps
        .filter((o) => o.status === "ACTIONABLE")
        .map((o) => ({ label: `${o.id} · ${o.symbol ?? o.pair}`, hint: `机会 ${fmt.signed(o.net)}`, hash: `#/opportunity/${o.id}` })),
    ],
    // 实时数据更新后重建索引（version 变化即触发）
    [status.version],
  );

  const items = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return needle ? pool.filter((i) => `${i.label} ${i.hint}`.toLowerCase().includes(needle)) : pool;
  }, [pool, q]);

  useEffect(() => {
    if (open) {
      setQ("");
      setIdx(0);
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowDown") {
        e.preventDefault();
        setIdx((i) => (items.length ? (i + 1) % items.length : 0));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setIdx((i) => (items.length ? (i - 1 + items.length) % items.length : 0));
      } else if (e.key === "Enter") {
        const item = items[idx];
        if (item) {
          onClose();
          location.hash = item.hash;
        }
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, items, idx, onClose]);

  if (!open) return null;

  return (
    <div className="cmdk">
      <div className="cmdk-scrim" onClick={onClose} />
      <div className="cmdk-box" role="dialog" aria-modal="true" aria-label="命令面板">
        <div className="cmdk-input">
          <Icon name="search" size={15} />
          <input
            ref={inputRef}
            value={q}
            onChange={(e) => {
              setQ(e.target.value);
              setIdx(0);
            }}
            placeholder="跳转到视图、机会、执行…"
            autoComplete="off"
            spellCheck={false}
            aria-label="搜索"
          />
          <kbd>ESC</kbd>
        </div>
        <div className="cmdk-list">
          {items.length === 0 && <div className="empty" style={{ margin: 6 }}>没有匹配项</div>}
          {items.map((item, i) => (
            <button
              key={`${item.hash}-${i}`}
              className={`cmdk-item${i === idx ? " is-active" : ""}`}
              onMouseEnter={() => setIdx(i)}
              onClick={() => {
                onClose();
                location.hash = item.hash;
              }}
            >
              {item.label}
              <small>{item.hint}</small>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
