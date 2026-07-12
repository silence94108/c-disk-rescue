import { computed, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  CapacityBreakdown,
  CleanableItem,
  CleanablesReport,
  CleanResult,
  MigratableItem,
  OrphanProfile,
  ScanProgress,
  ScanSummary,
} from "./api/types";
import {
  getCapacityBreakdown,
  getMigratables,
  getOrphanProfiles,
  onCleanProgress,
  onScanProgress,
  runClean,
  scanCleanables,
  startScan,
} from "./api";

/** 本次会话的扫描结果概要;整棵树在 Rust 侧,前端按需取 */
export const scanSummary = ref<ScanSummary | null>(null);

/** 启动时未完成搬家事务的自动恢复结果,概览页展示一次 */
export const recoverNotice = ref("");

const LAST_SCAN_KEY = "c-disk-rescue:last-scan";

export interface LastScan {
  at: number;
  totalBytes: number;
  /** 上次清理释放的字节数,概览「上次体检:释放了 X GB」用 */
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

/* ── 扫描流:状态放 store,体检中切页再切回不丢进度(侧栏改版后页面可自由切换) ── */

export const scanning = ref(false);
export const scanProgress = ref<ScanProgress | null>(null);

/** 体检:扫描 → 存概要 → 拉全报告数据;错误向上抛由页面安抚 */
export async function runScanFlow(): Promise<void> {
  if (scanning.value) return;
  scanning.value = true;
  scanProgress.value = null;
  let unlisten: UnlistenFn | null = null;
  try {
    unlisten = await onScanProgress((p) => {
      scanProgress.value = p;
    });
    const summary = await startScan();
    scanSummary.value = summary;
    saveLastScan({
      at: Date.now(),
      totalBytes: summary.totalBytes,
      freedBytes: loadLastScan()?.freedBytes,
    });
    // 报告整理失败不算扫描失败:垃圾清理页进入时会自动补拉
    await loadReportData().catch(() => {});
  } finally {
    unlisten?.();
    scanning.value = false;
  }
}

/* ── 体检报告数据:概览页(行动区数字/一键优化)与垃圾清理页共享同一份 ── */

export const cleanReport = ref<CleanablesReport | null>(null);
export const migratables = ref<MigratableItem[]>([]);
export const orphans = ref<OrphanProfile[]>([]);
export const breakdown = ref<CapacityBreakdown | null>(null);
export const checked = ref<Record<string, boolean>>({});

export function isDisabled(item: CleanableItem): boolean {
  if (item.guideOnly) return true;
  if (item.lockedBy.length > 0) return true;
  if (item.needsAdmin && !cleanReport.value?.isElevated) return true;
  return false;
}

/** 体检完成后拉全报告数据;分段容量条取数失败降级(breakdown 置 null,两段显示) */
export async function loadReportData() {
  const [clean, mig, orph, seg] = await Promise.all([
    scanCleanables(),
    getMigratables().catch(() => [] as MigratableItem[]),
    getOrphanProfiles().catch(() => [] as OrphanProfile[]),
    getCapacityBreakdown().catch(() => null),
  ]);
  cleanReport.value = clean;
  migratables.value = mig;
  orphans.value = orph;
  breakdown.value = seg;
  /* 默认勾选:放心删 + 有代价;谨慎级和被禁用项不勾(需求文档 F2 分级) */
  const next: Record<string, boolean> = {};
  for (const item of clean.items) {
    next[item.ruleId] = item.risk !== "caution" && !isDisabled(item);
  }
  checked.value = next;
}

export const cleanItems = computed(() => cleanReport.value?.items ?? []);
export const junkTotal = computed(() =>
  cleanItems.value.reduce((s, i) => s + i.sizeBytes, 0),
);
export const migrateTotal = computed(() =>
  migratables.value.reduce((s, i) => s + i.sizeBytes, 0),
);
/* 「预计可释放」只计垃圾 + 可搬家,大文件待用户决定不并入(设计规范 §3.2) */
export const releasable = computed(() => junkTotal.value + migrateTotal.value);

export const selectedItems = computed(() =>
  cleanItems.value.filter((i) => checked.value[i.ruleId] && !isDisabled(i)),
);
export const selectedBytes = computed(() =>
  selectedItems.value.reduce((s, i) => s + i.sizeBytes, 0),
);

/* ── 清理执行:概览「一键优化」与垃圾清理页「一键清理」是同一动作、同一状态 ── */

export const cleanPhase = ref<"idle" | "cleaning">("idle");
export const cleanProgressBytes = ref(0);

/** 清理勾选项;完成返回结果并后台刷新报告(已清理项归零消失),失败向上抛由页面安抚 */
export async function startCleanSelected(): Promise<CleanResult | null> {
  if (cleanPhase.value !== "idle" || selectedItems.value.length === 0) return null;
  cleanPhase.value = "cleaning";
  cleanProgressBytes.value = 0;
  let unlisten: UnlistenFn | null = null;
  try {
    unlisten = await onCleanProgress((p) => {
      cleanProgressBytes.value = p.freedBytes;
    });
    const result = await runClean(selectedItems.value.map((i) => i.ruleId));
    const last = loadLastScan();
    if (last) {
      saveLastScan({
        ...last,
        freedBytes: (last.freedBytes ?? 0) + result.freedBytes,
      });
    }
    loadReportData().catch(() => {});
    return result;
  } finally {
    unlisten?.();
    cleanPhase.value = "idle";
  }
}
