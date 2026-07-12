import { computed, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  BigFileInfo,
  CapacityBreakdown,
  CleanableItem,
  CleanablesReport,
  CleanResult,
  KnownFolderInfo,
  MigratableItem,
  MigrateCandidate,
  OrphanProfile,
  ScanProgress,
  ScanSummary,
} from "./api/types";
import {
  getBigFiles,
  getCapacityBreakdown,
  getMigratables,
  getMigrateCandidates,
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
    isStale.value = false; // 刚扫完是最新的
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
export const bigFiles = ref<BigFileInfo[]>([]);
export const candidates = ref<MigrateCandidate[]>([]);
export const knownFolders = ref<KnownFolderInfo[]>([]);
export const checked = ref<Record<string, boolean>>({});

/** 报告数据的产出时间;null = 本会话还没有任何报告 */
export const reportAt = ref<number | null>(null);
/** 当前报告是否来自「上次缓存」(重启恢复),而非本会话新体检 */
export const isStale = ref(false);

export function isDisabled(item: CleanableItem): boolean {
  if (item.guideOnly) return true;
  if (item.lockedBy.length > 0) return true;
  if (item.needsAdmin && !cleanReport.value?.isElevated) return true;
  return false;
}

/** 按分级规则重建默认勾选:放心删 + 有代价 默认勾,谨慎级与禁用项不勾 */
function rebuildChecked(clean: CleanablesReport) {
  const next: Record<string, boolean> = {};
  for (const item of clean.items) {
    next[item.ruleId] = item.risk !== "caution" && !isDisabled(item);
  }
  checked.value = next;
}

/**
 * 体检完成后拉全报告数据。依赖 arena 的几项(可搬家/分段/大文件/自选候选)
 * 在缓存态(重启后 Rust 树为空)会失败——所以本函数只在新体检后调用,
 * 缓存态改由 restoreSnapshot 从本地快照恢复。
 */
export async function loadReportData() {
  const [clean, mig, orph, seg, bigs, cand] = await Promise.all([
    scanCleanables(),
    getMigratables().catch(() => [] as MigratableItem[]),
    getOrphanProfiles().catch(() => [] as OrphanProfile[]),
    getCapacityBreakdown().catch(() => null),
    getBigFiles().catch(() => [] as BigFileInfo[]),
    getMigrateCandidates().catch(() => ({ candidates: [], knownFolders: [] })),
  ]);
  cleanReport.value = clean;
  migratables.value = mig;
  orphans.value = orph;
  breakdown.value = seg;
  bigFiles.value = bigs;
  candidates.value = cand.candidates;
  knownFolders.value = cand.knownFolders;
  rebuildChecked(clean);
  reportAt.value = Date.now();
  saveSnapshot();
}

/** 清理只影响垃圾项,单独刷新 cleanReport(缓存态也安全:scan_cleanables 不依赖 arena) */
async function refreshCleanables() {
  const clean = await scanCleanables();
  cleanReport.value = clean;
  rebuildChecked(clean);
  saveSnapshot();
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

/* ── 报告快照持久化:打开秒显示上次结果(用户选择:显示上次结果、手动重扫)── */

const REPORT_KEY = "c-disk-rescue:report";

interface ReportSnapshot {
  reportAt: number;
  summary: ScanSummary | null;
  cleanReport: CleanablesReport | null;
  migratables: MigratableItem[];
  orphans: OrphanProfile[];
  breakdown: CapacityBreakdown | null;
  bigFiles: BigFileInfo[];
  candidates: MigrateCandidate[];
  knownFolders: KnownFolderInfo[];
  checked: Record<string, boolean>;
}

function buildSnapshot(bigCap?: number): ReportSnapshot {
  return {
    reportAt: reportAt.value ?? Date.now(),
    summary: scanSummary.value,
    cleanReport: cleanReport.value,
    migratables: migratables.value,
    orphans: orphans.value,
    breakdown: breakdown.value,
    bigFiles: bigCap ? bigFiles.value.slice(0, bigCap) : bigFiles.value,
    candidates: candidates.value,
    knownFolders: knownFolders.value,
    checked: checked.value,
  };
}

/** 写本地快照。大文件多时可能超 localStorage 配额,降级只存前 300 个;仍失败则放弃(不影响功能) */
function saveSnapshot() {
  try {
    localStorage.setItem(REPORT_KEY, JSON.stringify(buildSnapshot()));
  } catch {
    try {
      localStorage.setItem(REPORT_KEY, JSON.stringify(buildSnapshot(300)));
    } catch {
      /* 配额仍不够就不存快照,下次打开退回手动体检 */
    }
  }
}

/** 启动时从本地快照恢复上次报告(缓存态)。恢复成功返回 true。 */
export function restoreSnapshot(): boolean {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(REPORT_KEY);
  } catch {
    return false;
  }
  if (!raw) return false;
  try {
    const s = JSON.parse(raw) as ReportSnapshot;
    if (!s.summary || !s.cleanReport) return false;
    scanSummary.value = s.summary;
    cleanReport.value = s.cleanReport;
    migratables.value = s.migratables ?? [];
    orphans.value = s.orphans ?? [];
    breakdown.value = s.breakdown ?? null;
    bigFiles.value = s.bigFiles ?? [];
    candidates.value = s.candidates ?? [];
    knownFolders.value = s.knownFolders ?? [];
    checked.value = s.checked ?? {};
    reportAt.value = s.reportAt ?? null;
    isStale.value = true; // 来自缓存,标记「上次结果」
    return true;
  } catch {
    return false;
  }
}

/* ── 报告项移除后同步快照:防止已删/已搬的项在重启后从缓存快照复现 ── */

export function dropBigFile(path: string) {
  bigFiles.value = bigFiles.value.filter((f) => f.path !== path);
  saveSnapshot();
}
export function dropMigratable(ruleId: string) {
  migratables.value = migratables.value.filter((m) => m.ruleId !== ruleId);
  saveSnapshot();
}
export function dropCandidate(id: string) {
  candidates.value = candidates.value.filter((c) => c.id !== id);
  saveSnapshot();
}
export function dropOrphan(path: string) {
  orphans.value = orphans.value.filter((o) => o.path !== path);
  saveSnapshot();
}

/* ── 清理执行:概览「一键优化」与垃圾清理页「一键清理」是同一动作、同一状态 ── */

export const cleanPhase = ref<"idle" | "cleaning">("idle");
export const cleanProgressBytes = ref(0);

/** 清理勾选项;完成后刷新垃圾列表(已清理项归零消失),失败向上抛由页面安抚 */
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
    // 只刷新垃圾项(清理不影响大文件/可搬家);缓存态下也安全
    refreshCleanables().catch(() => {});
    return result;
  } finally {
    unlisten?.();
    cleanPhase.value = "idle";
  }
}
