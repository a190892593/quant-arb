import { fmt } from "../lib/format";

export interface WaterfallStep {
  label: string;
  value: number;
  kind: "start" | "delta" | "total";
}

interface Bar extends WaterfallStep {
  from: number;
  to: number;
}

export function Waterfall({ steps, height = 168 }: { steps: WaterfallStep[]; height?: number }) {
  let run = 0;
  const bars: Bar[] = steps.map((s) => {
    if (s.kind === "start") {
      run = s.value;
      return { ...s, from: 0, to: s.value };
    }
    if (s.kind === "total") return { ...s, from: 0, to: s.value };
    const from = run;
    const to = run + s.value;
    run = to;
    return { ...s, from, to };
  });

  const top = Math.max(...bars.map((b) => Math.max(b.from, b.to)));
  const floor = Math.min(0, ...bars.map((b) => Math.min(b.from, b.to)));
  const span0 = top - floor || 1;
  const bottom = floor - span0 * 0.12;
  const span = top - bottom || 1;
  const pos = (v: number) => ((top - v) / span) * 100;

  return (
    <div className="wf" style={{ ["--wf-h" as string]: `${height}px` }}>
      {bars.map((b, i) => {
        const y1 = pos(Math.max(b.from, b.to));
        const y2 = pos(Math.min(b.from, b.to));
        const h = Math.max(1.5, y2 - y1);
        const kind =
          b.kind === "start" ? "start" : b.kind === "total" ? (b.value < 0 ? "total-neg" : "total") : b.value < 0 ? "neg" : "pos";
        const val = b.kind === "delta" ? fmt.signed(b.value) : fmt.num(b.value);
        const valTone = b.kind === "delta" ? (b.value < 0 ? " neg" : " pos") : "";
        return (
          <div className="wf-col" key={i}>
            <span className={`wf-val${valTone}`}>{val}</span>
            <div className="wf-track">
              <i className={`wf-bar wf-bar--${kind}`} style={{ top: `${y1.toFixed(2)}%`, height: `${h.toFixed(2)}%` }} />
            </div>
            <span className="wf-label">{b.label}</span>
            {i < bars.length - 1 && <b className="wf-link" style={{ top: `${pos(b.to).toFixed(2)}%` }} />}
          </div>
        );
      })}
    </div>
  );
}
