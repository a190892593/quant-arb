import { useSyncExternalStore } from "react";
import { data as demo } from "./dataset";
import { executeOpportunity, fetchLive, resetPaper, riskPause, runAudit, scannerAction } from "./api";
import { scanStore } from "./store";
import type { DataModel } from "../types";

/**
 * 数据边界：页面只 `import { data } from "../data"`。
 * 演示数据在模块加载时就位，`refresh()` 用后端返回的真实数据**原地**覆盖同名集合，
 * 因此视图里模块级的 `const ROWS = data.opps` 引用依然有效（不会指向旧数组）。
 */
export const data: DataModel = demo;

export interface DataStatus {
  live: boolean;
  loading: boolean;
  loadedAt: number | null;
  error: string | null;
  version: number;
}

let status: DataStatus = { live: false, loading: false, loadedAt: null, error: null, version: 0 };
const listeners = new Set<() => void>();

const emit = () => listeners.forEach((fn) => fn());

function setStatus(next: Partial<DataStatus>) {
  status = { ...status, ...next, version: status.version + 1 };
  emit();
}

export function subscribeData(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export const getDataStatus = (): DataStatus => status;

export function useDataStatus(): DataStatus {
  return useSyncExternalStore(subscribeData, getDataStatus, getDataStatus);
}

/**
 * 原地合并：数组清空后填充、对象逐字段覆盖，其余直接赋值。
 * 这样 Accounts/Executions 等模块级缓存的数组引用不会失效。
 */
function mergeInto(target: Record<string, unknown>, patch: Record<string, unknown>): void {
  for (const [key, next] of Object.entries(patch)) {
    if (next === undefined) continue;
    const current = target[key];
    if (Array.isArray(next) && Array.isArray(current)) {
      current.length = 0;
      current.push(...next);
    } else if (
      next &&
      typeof next === "object" &&
      !Array.isArray(next) &&
      current &&
      typeof current === "object" &&
      !Array.isArray(current)
    ) {
      Object.assign(current, next);
    } else {
      target[key] = next;
    }
  }
}

let inflight: Promise<void> | null = null;

/** 拉取后端快照并合并进 data；并发调用合并为一次请求。 */
export function refresh(): Promise<void> {
  if (inflight) return inflight;
  setStatus({ loading: true });
  inflight = fetchLive()
    .then(({ patch }) => {
      mergeInto(data as unknown as Record<string, unknown>, patch as Record<string, unknown>);
      // 后端扫描状态 → 两个扫描页共享的开关
      scanStore.setBoth(data.scanner.state === "RUNNING");
      setStatus({ live: true, loading: false, loadedAt: Date.now(), error: null });
    })
    .catch((err: unknown) => {
      setStatus({
        live: false,
        loading: false,
        error: err instanceof Error ? err.message : "接口不可用",
      });
    })
    .finally(() => {
      inflight = null;
    });
  return inflight;
}

let pollTimer: number | null = null;

/** 启动轮询（幂等）。页面隐藏时暂停请求，回到前台立即刷新一次。 */
export function startPolling(intervalMs = 3000): void {
  if (pollTimer !== null) return;
  void refresh();
  pollTimer = window.setInterval(() => {
    if (document.hidden) return;
    void refresh();
  }, intervalMs);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) void refresh();
  });
}

export function stopPolling(): void {
  if (pollTimer !== null) {
    window.clearInterval(pollTimer);
    pollTimer = null;
  }
}

/** 写操作后立即刷新，保证界面与后端账本一致 */
async function afterWrite(fn: () => Promise<void>): Promise<void> {
  await fn();
  await refresh();
}

export const actions = {
  /** 启动/停止扫描（后端只有一个扫描引擎，两个扫描页同步） */
  async setScan(on: boolean): Promise<void> {
    scanStore.setBoth(on);
    try {
      await afterWrite(() => scannerAction(on ? "start" : "stop"));
    } catch (e) {
      scanStore.setBoth(!on);
      throw e;
    }
  },
  setPaused: (paused: boolean) => afterWrite(() => riskPause(paused)),
  execute: (id: string) => afterWrite(() => executeOpportunity(id)),
  reset: () => afterWrite(() => resetPaper()),
  audit: (dataset?: string) => runAudit(dataset),
};

export { executeOpportunity, resetPaper, riskPause, scannerAction };
