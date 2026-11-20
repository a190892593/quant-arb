const MINUS = "\u2212";

const nf = (dp: number) =>
  new Intl.NumberFormat("en-US", { minimumFractionDigits: dp, maximumFractionDigits: dp });

export const fmt = {
  num(n: number | null | undefined, dp = 2): string {
    if (n === null || n === undefined || Number.isNaN(n)) return "—";
    return nf(dp).format(n);
  },
  signed(n: number | null | undefined, dp = 2): string {
    if (n === null || n === undefined || Number.isNaN(n)) return "—";
    const s = nf(dp).format(Math.abs(n));
    if (n > 0) return "+" + s;
    if (n < 0) return MINUS + s;
    return nf(dp).format(0);
  },
  pct(n: number, dp = 2): string {
    return fmt.num(n, dp) + "%";
  },
  age(ms: number): string {
    if (ms < 1000) return Math.round(ms) + " ms";
    if (ms < 60_000) return (ms / 1000).toFixed(ms < 10_000 ? 2 : 1) + " s";
    const m = Math.floor(ms / 60_000);
    if (m < 60) return m + "m " + Math.round((ms % 60_000) / 1000) + "s";
    return Math.floor(m / 60) + "h " + (m % 60) + "m";
  },
  compact(n: number): string {
    const v = Math.abs(n);
    if (v >= 1e9) return (n / 1e9).toFixed(2) + "B";
    if (v >= 1e6) return (n / 1e6).toFixed(2) + "M";
    if (v >= 1e4) return (n / 1e3).toFixed(1) + "k";
    return fmt.num(n, 0);
  },
  /** ISO 时间戳 → HH:MM:SS（本地时区），用于「出现时间」列 */
  hhmmss(iso: string | null | undefined): string {
    if (!iso) return "—";
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "—";
    const p = (v: number) => String(v).padStart(2, "0");
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  },
  clock(d: Date): string {
    const p = (v: number) => String(v).padStart(2, "0");
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  },
};

/** css class for a signed value */
export const signClass = (n: number): string => (n > 0 ? "pos" : n < 0 ? "neg" : "muted");

export const toneClass = (pct: number): string => (pct >= 85 ? "bad" : pct >= 60 ? "warn" : "");
