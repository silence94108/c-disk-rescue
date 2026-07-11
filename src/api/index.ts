import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  CleanablesReport,
  CleanProgress,
  CleanResult,
  DiskInfo,
  LockStatus,
  MigratableItem,
  ScanProgress,
  ScanSummary,
  TreeNode,
} from "./types";

export const getDisks = () => invoke<DiskInfo[]>("get_disks");

export const startScan = (root?: string) =>
  invoke<ScanSummary>("start_scan", { root: root ?? null });

export const cancelScan = () => invoke<void>("cancel_scan");

export const getChildren = (nodeId: number) =>
  invoke<TreeNode[]>("get_children", { nodeId });

export const getMigratables = () => invoke<MigratableItem[]>("get_migratables");

export const scanCleanables = () => invoke<CleanablesReport>("scan_cleanables");

export const checkLocks = (ruleIds: string[]) =>
  invoke<LockStatus[]>("check_locks", { ruleIds });

export const runClean = (ruleIds: string[]) =>
  invoke<CleanResult>("run_clean", { ruleIds });

export const cancelClean = () => invoke<void>("cancel_clean");

export const onScanProgress = (
  cb: (p: ScanProgress) => void,
): Promise<UnlistenFn> => listen<ScanProgress>("scan:progress", (e) => cb(e.payload));

export const onCleanProgress = (
  cb: (p: CleanProgress) => void,
): Promise<UnlistenFn> => listen<CleanProgress>("clean:progress", (e) => cb(e.payload));
