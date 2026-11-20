import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

/**
 * 通用右侧抽屉：
 * - 默认宽度为屏幕一半，左缘可拖拽调宽（双击复位）
 * - 宽度按 storageKey 记忆，下次打开沿用
 * - Esc / 遮罩点击 / 红色 X 均可关闭
 */
export function Drawer({
  head,
  children,
  footer,
  onClose,
  ariaLabel,
  storageKey = "drawer",
  className = "",
  bodyClassName = "",
}: {
  head: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  onClose: () => void;
  ariaLabel: string;
  storageKey?: string;
  className?: string;
  bodyClassName?: string;
}) {
  const storeKey = `arbx.drawer.w.${storageKey}`;
  const [width, setWidth] = useState<number>(() => {
    const saved = Number(localStorage.getItem(storeKey));
    if (Number.isFinite(saved) && saved > 0) return saved;
    return Math.round(window.innerWidth / 2);
  });
  const [dragging, setDragging] = useState(false);
  const dragRef = useRef(false);

  const clamp = (w: number) =>
    Math.max(380, Math.min(Math.round(window.innerWidth * 0.94), Math.round(w)));

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    dragRef.current = true;
    setDragging(true);
    (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
  }, []);

  useEffect(() => {
    const move = (e: PointerEvent) => {
      if (!dragRef.current) return;
      setWidth(clamp(window.innerWidth - e.clientX));
    };
    const up = () => {
      if (!dragRef.current) return;
      dragRef.current = false;
      setDragging(false);
      setWidth((w) => {
        localStorage.setItem(storeKey, String(w));
        return w;
      });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [storeKey]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const resetWidth = () => {
    const w = Math.round(window.innerWidth / 2);
    setWidth(w);
    localStorage.setItem(storeKey, String(w));
  };

  return (
    <>
      <div className="drawer-scrim" onClick={onClose} />
      <aside
        className={`drawer ${className}`.trim()}
        role="dialog"
        aria-label={ariaLabel}
        style={{ width }}
      >
        <div
          className={`drawer-resize${dragging ? " is-active" : ""}`}
          onPointerDown={onPointerDown}
          onDoubleClick={resetWidth}
          title="拖动调整宽度，双击复位为屏幕一半"
          role="separator"
          aria-orientation="vertical"
        />
        <header className="drawer-head">
          <div className="drawer-head-main">{head}</div>
          <button className="drawer-x" onClick={onClose} aria-label="关闭">
            <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
              <path
                d="M6 6l12 12M18 6L6 18"
                stroke="currentColor"
                strokeWidth="2.4"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </header>
        <div className={`drawer-body ${bodyClassName}`.trim()}>{children}</div>
        {footer && <footer className="drawer-foot">{footer}</footer>}
      </aside>
    </>
  );
}
