import { useEffect, useRef, useState } from "react";
import { fmt } from "../lib/format";

export interface ScanEvent {
  tone: "ok" | "warn" | "bad" | "info";
  text: string;
}

export interface ScanStripProps {
  running: boolean;
  subjects: string[];
  events: ScanEvent[];
  quotes: number;
  quotesLabel?: string;
  pools: number;
  poolLabel?: string;
  block: number;
  latency: number;
  latencyLabel?: string;
}

const DOT: Record<string, string> = {
  ok: "dot--ok",
  warn: "dot--warn",
  bad: "dot--bad",
  info: "dot--idle",
};

export function ScanStrip({ running, subjects, events, quotes, quotesLabel = "quotes/s", pools, poolLabel = "池", block, latency, latencyLabel = "扫描延迟" }: ScanStripProps) {
  const [subject, setSubject] = useState(0);
  const [event, setEvent] = useState(0);
  const [fading, setFading] = useState(false);
  const [q, setQ] = useState(quotes);
  const [lat, setLat] = useState(latency);
  const fadeTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!running) return;
    const t1 = window.setInterval(() => setSubject((i) => (i + 1) % subjects.length), 2600);
    const t2 = window.setInterval(() => {
      setFading(true);
      fadeTimer.current = window.setTimeout(() => {
        setEvent((i) => (i + 1) % events.length);
        setFading(false);
      }, 180);
    }, 3200);
    const t3 = window.setInterval(() => {
      setQ(Math.round(quotes * (0.96 + Math.random() * 0.09)));
      setLat(Math.max(8, latency + Math.round((Math.random() - 0.5) * 14)));
    }, 1500);
    return () => {
      clearInterval(t1);
      clearInterval(t2);
      clearInterval(t3);
      if (fadeTimer.current) clearTimeout(fadeTimer.current);
      setFading(false);
    };
  }, [running, subjects.length, events.length, quotes, latency]);

  const e = events[event] ?? { tone: "info" as const, text: "" };

  return (
    <div className={`scanfx${running ? "" : " is-idle"}`} role="status" aria-live="polite">
      {running && <i className="scanfx-beam" aria-hidden="true" />}
      <span className="scanfx-state">
        <span className={`scan-pulse${running ? "" : " scan-pulse--off"}`} />
        <b>{running ? "SEARCHING" : "STOPPED"}</b>
      </span>
      <span className="scanfx-now">
        <span className="eyebrow">NOW</span>
        <span id="fx-subject">{subjects[subject]}</span>
      </span>
      <span className="scanfx-metrics">
        <span><i>{quotesLabel}</i><b>{fmt.num(q, 0)}</b></span>
        <span><i>{poolLabel}</i><b>{fmt.num(pools, 0)}</b></span>
        <span><i>最新区块</i><b>{fmt.num(block, 0)}</b></span>
        <span><i>{latencyLabel}</i><b>{lat} ms</b></span>
      </span>
      <span className={`scanfx-event${fading ? " is-out" : ""}`}>
        <span className={`dot ${DOT[e.tone]}`} />
        <span>{e.text}</span>
      </span>
      {running && (
        <span className="scanfx-track" aria-hidden="true">
          <i />
        </span>
      )}
    </div>
  );
}
