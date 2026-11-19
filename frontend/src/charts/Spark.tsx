export function Spark({
  values,
  w = 78,
  h = 20,
  tone = "accent",
}: {
  values: number[];
  w?: number;
  h?: number;
  tone?: "accent" | "pos" | "neg";
}) {
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const lo = min - span * 0.18;
  const hi = max + span * 0.18;
  const pts = values.map((v, i) => {
    const x = (i / Math.max(1, values.length - 1)) * w;
    const y = h - ((v - lo) / (hi - lo)) * h;
    return `${i === 0 ? "M" : "L"}${x.toFixed(2)} ${y.toFixed(2)}`;
  });
  const color = tone === "pos" ? "var(--pos)" : tone === "neg" ? "var(--neg)" : "var(--accent)";
  return (
    <svg className="spark" viewBox={`0 0 ${w} ${h}`} width={w} height={h} fill="none" aria-hidden="true">
      <path d={pts.join(" ")} stroke={color} strokeWidth={1.2} strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}
