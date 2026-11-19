import { useEffect, useState } from "react";

type Variant = "info" | "warn" | "bad";
interface ToastItem {
  id: number;
  msg: string;
  sub?: string;
  variant: Variant;
}

let seq = 0;
const listeners = new Set<(items: ToastItem[]) => void>();
let items: ToastItem[] = [];

function emit() {
  listeners.forEach((fn) => fn(items));
}

export function toast(msg: string, opts: { sub?: string; variant?: Variant; ttl?: number } = {}) {
  const item: ToastItem = { id: ++seq, msg, sub: opts.sub, variant: opts.variant ?? "info" };
  items = [...items, item];
  emit();
  setTimeout(() => {
    items = items.filter((x) => x.id !== item.id);
    emit();
  }, opts.ttl ?? 3600);
}

export function Toaster() {
  const [list, setList] = useState<ToastItem[]>(items);
  useEffect(() => {
    listeners.add(setList);
    return () => {
      listeners.delete(setList);
    };
  }, []);

  return (
    <div className="toasts" aria-live="polite">
      {list.map((t) => (
        <div key={t.id} className={`toast${t.variant === "info" ? "" : " toast--" + t.variant}`}>
          {t.msg}
          {t.sub && <small>{t.sub}</small>}
        </div>
      ))}
    </div>
  );
}
