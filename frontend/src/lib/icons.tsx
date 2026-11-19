import type { JSX } from "react";

const PATHS: Record<string, JSX.Element> = {
  overview: (
    <>
      <rect x="2.2" y="2.2" width="5" height="5" rx=".5" />
      <rect x="8.8" y="2.2" width="5" height="5" rx=".5" />
      <rect x="2.2" y="8.8" width="5" height="5" rx=".5" />
      <rect x="8.8" y="8.8" width="5" height="5" rx=".5" />
    </>
  ),
  scanner: (
    <>
      <circle cx="8" cy="8" r="5.6" />
      <path d="M8 8 11.6 5.2" />
      <circle cx="8" cy="8" r="1.2" fill="currentColor" stroke="none" />
    </>
  ),
  route: <path d="M2.2 5.2h8.4l-2-2M13.8 10.8H5.4l2 2" />,
  wallet: (
    <>
      <rect x="1.8" y="4" width="12.4" height="9" rx="1.2" />
      <path d="M1.8 6.6h9.4M11.6 9.4h2.6" />
    </>
  ),
  chart: (
    <>
      <path d="M2 13V3M2 13h12" />
      <path d="M4.4 10.2 7 7.4l2.2 1.8L13.4 4" />
    </>
  ),
  audit: (
    <>
      <rect x="2" y="3" width="12" height="3.4" rx=".6" />
      <path d="M3.2 6.4v6.4c0 .6.4 1 1 1h7.6c.6 0 1-.4 1-1V6.4M6.6 9h2.8" />
    </>
  ),
  ledger: (
    <>
      <path d="M3 2.4h8.6c.9 0 1.4.5 1.4 1.4v9.8H4.4c-.8 0-1.4-.6-1.4-1.4V2.4Z" />
      <path d="M3 11.6h10M5.4 5.4h5M5.4 7.8h5" />
    </>
  ),
  shield: (
    <>
      <path d="M8 1.8 13.2 4v4.3c0 3-2.2 5.2-5.2 6.1-3-1-5.2-3.1-5.2-6.1V4L8 1.8Z" />
      <path d="m5.9 8 1.6 1.6 2.9-3" />
    </>
  ),
  system: (
    <>
      <rect x="2" y="2.4" width="12" height="4.4" rx=".8" />
      <rect x="2" y="9.2" width="12" height="4.4" rx=".8" />
      <path d="M4.2 4.6h.01M4.2 11.4h.01" />
    </>
  ),
  search: (
    <>
      <circle cx="7" cy="7" r="4.2" />
      <path d="m10.2 10.2 3.2 3.2" />
    </>
  ),
  target: (
    <>
      <circle cx="8" cy="8" r="4.8" />
      <path d="M8 1.4v2.2M8 12.4v2.2M1.4 8h2.2M12.4 8h2.2" />
    </>
  ),
  play: <path d="M5 3.4v9.2L12.4 8 5 3.4Z" />,
  pause: <path d="M5.6 3.6v8.8M10.4 3.6v8.8" />,
  refresh: (
    <>
      <path d="M13.2 6.4A5.4 5.4 0 1 0 13 9.4" />
      <path d="M13.4 2.8v3.8H9.6" />
    </>
  ),
  external: (
    <>
      <path d="M9.4 3.2h3.4v3.4M12.8 3.2 7.6 8.4" />
      <path d="M12.4 9.2v3.2H3.4V3.6h3.2" />
    </>
  ),
  chevron: <path d="m6 4 4 4-4 4" />,
  check: <path d="m3.4 8.4 3 3 6.2-7" />,
  x: <path d="m4 4 8 8M12 4l-8 8" />,
  alert: (
    <>
      <path d="M8 2.6 14 13H2L8 2.6Z" />
      <path d="M8 6.4v3.2M8 11.2h.01" />
    </>
  ),
  sim: (
    <>
      <path d="M4 3.2 12.4 8 4 12.8V3.2Z" />
      <path d="M2 8h1.2M12.8 8H14" />
    </>
  ),
  pools: (
    <>
      <path d="M8 2.4 13.6 5.4 8 8.4 2.4 5.4 8 2.4Z" />
      <path d="M2.4 8.4 8 11.4l5.6-3" />
      <path d="M2.4 11.2 8 14.2l5.6-3" />
    </>
  ),
  data: <path d="M2 8h2.2l1.6-4 2.4 8 1.8-5.2 1.4 1.2H14" />,
  contract: (
    <>
      <path d="M6 4.6 3 8l3 3.4M10 4.6 13 8l-3 3.4" />
      <path d="m9 3.4-2 9.2" />
    </>
  ),
  triangle: (
    <>
      <circle cx="8" cy="3.4" r="1.5" />
      <circle cx="3.2" cy="12.2" r="1.5" />
      <circle cx="12.8" cy="12.2" r="1.5" />
      <path d="M7.2 4.8 4 10.9M8.8 4.8 12 10.9M4.8 12.2h6.4" />
    </>
  ),
  cross: (
    <>
      <rect x="2" y="2.8" width="4.6" height="4.6" rx=".6" />
      <rect x="9.4" y="8.6" width="4.6" height="4.6" rx=".6" />
      <path d="M6.6 5.1h4a2.2 2.2 0 0 1 2.2 2.2v1.3M9.4 10.9h-4a2.2 2.2 0 0 1-2.2-2.2V7.4" />
    </>
  ),
  hedge: (
    <>
      <path d="M2.4 4.6h7.4M9.8 4.6 8 3M9.8 4.6 8 6.2" />
      <path d="M13.6 11.4H6.2M6.2 11.4l1.8-1.6M6.2 11.4l1.8 1.6" />
    </>
  ),
};

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.25}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {PATHS[name]}
    </svg>
  );
}
