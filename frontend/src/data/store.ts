import { useSyncExternalStore } from "react";

/**
 * 扫描开关的本地镜像。后端只有一个扫描引擎，两个扫描页共享同一状态：
 * 真正的 start/stop 由 `actions.setScan()` 调用接口后再同步这里。
 */
export interface ScanState {
  scanDex: boolean;
  scanCex: boolean;
}

let state: ScanState = { scanDex: true, scanCex: true };
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((fn) => fn());

export const scanStore = {
  get: (): ScanState => state,
  subscribe: (fn: () => void) => {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  set(key: "scanDex" | "scanCex", value: boolean) {
    if (state[key] === value) return;
    state = { ...state, [key]: value };
    emit();
  },
  setBoth(value: boolean) {
    if (state.scanDex === value && state.scanCex === value) return;
    state = { scanDex: value, scanCex: value };
    emit();
  },
};

export function useScanState(): ScanState {
  return useSyncExternalStore(scanStore.subscribe, scanStore.get, scanStore.get);
}
