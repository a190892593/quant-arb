import { useId, useMemo, useRef, useState, type PointerEvent } from "react";

export interface LineChartProps {
  values: number[];
  height?: number;
  baseline?: number;
  tone?: "accent" | "pos" | "neg";
  fill?: boolean;
  fillToBaseline?: boolean;
  yFormat?: (v: number) => string;
  hoverFormat?: (v: number, i: number) => string;
  hoverLabel?: (i: number) => string;
  xLabels?: { text: string; at: number }[];
}

const TONE_VAR: Record<string, string> = {
  accent: "var(--accent)",
  pos: "var(--pos)",
  neg: "var(--neg)",
};

export function LineChart({
  values,
  height = 220,
  baseline,
  tone = "accent",
  fill = true,
  fillToBaseline = false,
  yFormat = (v) => Math.round(v).toLocaleString("en-US"),
  hoverFormat,
  hoverLabel,
  xLabels = [],
}: LineChartProps) {
  const gid = useId().replace(/:/g, "");
  if (values.length < 2) return <div className="empty" style={{ height }}>暂无序列数据</div>;
  const plotRef = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<number | null>(null);

  const geom = useMemo(() => {
    const W = 1000;
    const H = height;
    const min = Math.min(...values);
    const max = Math.max(...values);
    const lo0 = baseline !== undefined ? Math.min(min, baseline) : min;
    const hi0 = baseline !== undefined ? Math.max(max, baseline) : max;
    const span = hi0 - lo0 || 1;
    const lo = lo0 - span * 0.1;
    const hi = hi0 + span * 0.1;
    const x = (i: number) => (i / Math.max(1, values.length - 1)) * W;
    const y = (v: number) => H - ((v - lo) / (hi - lo)) * H;
    const pts = values.map((v, i) => [x(i), y(v)] as const);
    const line = pts.map((p, i) => `${i === 0 ? "M" : "L"}${p[0].toFixed(2)} ${p[1].toFixed(2)}`).join(" ");
    const areaBase = fillToBaseline && baseline !== undefined ? y(baseline) : H;
    const area = `${line} L${W} ${areaBase} L0 ${areaBase} Z`;
    const levels = [0, 1, 2, 3, 4].map((i) => hi - ((hi - lo) * i) / 4);
    return { W, H, lo, hi, x, y, line, area, levels, areaBase };
  }, [values, height, baseline, fillToBaseline]);

  const color = TONE_VAR[tone];

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    const el = plotRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const rel = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    setHover(Math.round(rel * (values.length - 1)));
  };

  const hx = hover !== null ? geom.x(hover) : 0;
  const hy = hover !== null ? geom.y(values[hover]) : 0;
  const plotW = plotRef.current?.clientWidth ?? 600;

  return (
    <div className="chart" style={{ ["--chart-h" as string]: `${height}px` }}>
      <div className="chart-axis">
        {geom.levels.map((v, i) => (
          <span key={i}>{yFormat(v)}</span>
        ))}
      </div>
      <div
        className="chart-plot"
        ref={plotRef}
        onPointerMove={onMove}
        onPointerLeave={() => setHover(null)}
      >
        <svg viewBox={`0 0 ${geom.W} ${geom.H}`} preserveAspectRatio="none" aria-hidden="true">
          <defs>
            <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={color} stopOpacity={0.22} />
              <stop offset="100%" stopColor={color} stopOpacity={0} />
            </linearGradient>
          </defs>
          {geom.levels.map((v, i) => (
            <line
              key={i}
              x1={0}
              y1={geom.y(v)}
              x2={geom.W}
              y2={geom.y(v)}
              stroke="var(--line-1)"
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
              strokeDasharray={i === 4 ? "0" : "2 4"}
            />
          ))}
          {baseline !== undefined && (
            <line
              x1={0}
              y1={geom.y(baseline)}
              x2={geom.W}
              y2={geom.y(baseline)}
              stroke="var(--ink-4)"
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
              strokeDasharray="4 4"
            />
          )}
          {fill && <path d={geom.area} fill={`url(#${gid})`} />}
          <path
            d={geom.line}
            stroke={color}
            strokeWidth={1.5}
            fill="none"
            vectorEffect="non-scaling-stroke"
            strokeLinejoin="round"
          />
          {hover !== null && (
            <g>
              <line
                x1={hx}
                y1={0}
                x2={hx}
                y2={geom.H}
                stroke="var(--ink-2)"
                strokeWidth={1}
                vectorEffect="non-scaling-stroke"
                strokeDasharray="2 3"
              />
              <circle
                cx={hx}
                cy={hy}
                r={3}
                fill="var(--bg-0)"
                stroke={color}
                strokeWidth={1.5}
                vectorEffect="non-scaling-stroke"
              />
            </g>
          )}
        </svg>
        <div className="chart-xlabels">
          {xLabels.map((l, i) => (
            <span key={i} style={{ left: `${l.at}%` }}>
              {l.text}
            </span>
          ))}
        </div>
        {hover !== null && (
          <div
            className="chart-tip"
            style={{
              left: Math.min(plotW - 110, Math.max(4, (hx / geom.W) * plotW + 10)),
              top: Math.max(4, (hy / geom.H) * height - 46),
            }}
          >
            <b>{hoverFormat ? hoverFormat(values[hover], hover) : values[hover].toFixed(2)}</b>
            <i>{hoverLabel ? hoverLabel(hover) : `第 ${hover + 1} 日`}</i>
          </div>
        )}
      </div>
    </div>
  );
}
