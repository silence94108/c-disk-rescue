import { ref } from "vue";
import type { ScanSummary } from "./api/types";

/** 本次会话的扫描结果概要;整棵树在 Rust 侧,前端按需取 */
export const scanSummary = ref<ScanSummary | null>(null);

const LAST_SCAN_KEY = "c-disk-rescue:last-scan";

export interface LastScan {
  at: number;
  totalBytes: number;
}

export function loadLastScan(): LastScan | null {
  try {
    const raw = localStorage.getItem(LAST_SCAN_KEY);
    return raw ? (JSON.parse(raw) as LastScan) : null;
  } catch {
    return null;
  }
}

export function saveLastScan(entry: LastScan) {
  localStorage.setItem(LAST_SCAN_KEY, JSON.stringify(entry));
}
