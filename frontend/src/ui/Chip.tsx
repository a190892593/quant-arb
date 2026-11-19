import type { ReactNode } from "react";

const CHIP_VARIANT: Record<string, string> = {
  ACTIONABLE: "accent",
  WATCH: "warn",
  REJECTED: "bad",
  EXPIRED: "idle",
  EXECUTED: "ok",
  PASS: "ok",
  PAPER: "warn",
  VALID: "ok",
  STALE: "bad",
  INVALID: "bad",
  ENABLED: "ok",
  DISABLED: "idle",
};

const CHIP_LABEL: Record<string, string> = {
  ACTIONABLE: "可执行",
  WATCH: "观察",
  REJECTED: "已拒绝",
  EXPIRED: "已过期",
  EXECUTED: "已执行",
};

export function Chip({
  children,
  variant = "idle",
  dot = false,
}: {
  children: ReactNode;
  variant?: "ok" | "bad" | "warn" | "accent" | "idle" | "plain";
  dot?: boolean;
}) {
  const dotTone = variant === "ok" ? "ok" : variant === "bad" ? "bad" : variant === "warn" ? "warn" : "idle";
  return (
    <span className={`chip chip--${variant}`}>
      {dot && <span className={`dot dot--${dotTone}`} />}
      {children}
    </span>
  );
}

export function StatusChip({ status }: { status: string }) {
  const variant = (CHIP_VARIANT[status] ?? "idle") as "ok" | "bad" | "warn" | "accent" | "idle";
  return (
    <Chip variant={variant} dot>
      {CHIP_LABEL[status] ?? status}
    </Chip>
  );
}

export function QualityChip({ quality }: { quality: string }) {
  const variant = quality === "A" ? "ok" : quality === "B" ? "warn" : "bad";
  return <Chip variant={variant}>Q{quality}</Chip>;
}
