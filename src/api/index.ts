import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  BigFileInfo,
  CleanablesReport,
  CleanProgress,
  CleanResult,
  DiskInfo,
  LockStatus,
  MigratableItem,
  MigrateProgress,
  MigrateRecord,
  MigrateResult,
  ScanProgress,
  ScanSummary,
  TargetDisk,
  TreeNode,
} from "./types";

export const getDisks = () => invoke<DiskInfo[]>("get_disks");

export const startScan = (root?: string) =>
  invoke<ScanSummary>("start_scan", { root: root ?? null });

export const cancelScan = () => invoke<void>("cancel_scan");

export const getChildren = (nodeId: number) =>
  invoke<TreeNode[]>("get_children", { nodeId });

export const getMigratables = () => invoke<MigratableItem[]>("get_migratables");

export const getBigFiles = () => invoke<BigFileInfo[]>("get_big_files");

export const deleteBigFile = (path: string) =>
  invoke<void>("delete_big_file", { path });

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

export const getMigrateTargets = () => invoke<TargetDisk[]>("get_migrate_targets");

export const startMigrate = (ruleId: string, targetRoot: string) =>
  invoke<MigrateResult>("start_migrate", { ruleId, targetRoot });

export const cancelMigrate = () => invoke<void>("cancel_migrate");

export const getMigrations = () => invoke<MigrateRecord[]>("get_migrations");

export const confirmMigration = (ruleId: string) =>
  invoke<void>("confirm_migration", { ruleId });

export const revertMigration = (ruleId: string) =>
  invoke<void>("revert_migration", { ruleId });

/** 「帮我退出」:优雅关闭锁定进程,返回超时仍在锁定的软件名(空 = 成功) */
export const requestClose = (ruleId: string) =>
  invoke<string[]>("request_close", { ruleId });

export const recoverPendingMigration = () =>
  invoke<string | null>("recover_pending_migration");

export const onMigrateProgress = (
  cb: (p: MigrateProgress) => void,
): Promise<UnlistenFn> =>
  listen<MigrateProgress>("migrate:progress", (e) => cb(e.payload));
