import { ref } from "vue";
import type { ScanSummary } from "./api/types";

/** 本次会话的扫描结果概要;整棵树在 Rust 侧,前端按需取 */
export const scanSummary = ref<ScanSummary | null>(null);

/** 启动时未完成搬家事务的自动恢复结果,首页展示一次 */
export const recoverNotice = ref("");

const LAST_SCAN_KEY = "c-disk-rescue:last-scan";

export interface LastScan {
  at: number;
  totalBytes: number;
  /** 上次清理释放的字节数,首页「上次体检:释放了 X GB」用 */
  freedBytes?: number;
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
