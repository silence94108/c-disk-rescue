/** 前后端契约 —— 与 src-tauri 各模块的 serde(rename_all = "camelCase")一一对应 */

export interface DiskInfo {
  mountPoint: string;
  totalBytes: number;
  freeBytes: number;
  isRemovable: boolean;
  isSystem: boolean;
}

export interface ScanProgress {
  scannedFiles: number;
  scannedBytes: number;
  currentPath: string;
}

export interface ScanSummary {
  rootId: number;
  totalBytes: number;
  totalFiles: number;
  dirCount: number;
  /** 权限不足被跳过的条目数,>0 时标注「部分系统区域未扫描」 */
  deniedEntries: number;
  elapsedMs: number;
}

export type Risk = "safe" | "cost" | "caution";
export type RuleAction = "clean" | "migrate" | "guide";

export interface RuleTag {
  displayName: string;
  explain: string;
  risk: Risk;
  action: RuleAction;
}

export interface TreeNode {
  id: number;
  name: string;
  path: string;
  sizeBytes: number;
  fileCount: number;
  hasChildren: boolean;
  isReparse: boolean;
  reparseTarget: string | null;
  rule: RuleTag | null;
}

export interface MigratableItem {
  ruleId: string;
  displayName: string;
  explain: string;
  path: string;
  sizeBytes: number;
}

export interface CleanableItem {
  ruleId: string;
  displayName: string;
  explain: string;
  risk: Risk;
  needsAdmin: boolean;
  path: string;
  sizeBytes: number;
  fileCount: number;
  /** 正锁定该项文件的软件友好名(Restart Manager 检出),非空则置灰并提示退出 */
  lockedBy: string[];
}

export interface CleanablesReport {
  items: CleanableItem[];
  /** 当前进程是否已提权,决定 needsAdmin 项能否执行 */
  isElevated: boolean;
}

export interface LockStatus {
  ruleId: string;
  lockedBy: string[];
}

export interface CleanProgress {
  ruleId: string;
  freedBytes: number;
  deletedFiles: number;
}

export interface SkippedRule {
  ruleId: string;
  reason: string;
}

export interface CleanResult {
  freedBytes: number;
  deletedFiles: number;
  /** 被占用等原因跳过的文件数(容错设计,不算失败) */
  failedFiles: number;
  skipped: SkippedRule[];
  logPath: string | null;
}

export interface TargetDisk {
  mountPoint: string;
  freeBytes: number;
  isNtfs: boolean;
  recommended: boolean;
}

export interface MigrateProgress {
  copiedBytes: number;
  totalBytes: number;
  currentFile: string;
}

export interface MigrateRecord {
  ruleId: string;
  displayName: string;
  /** 原位置(现为联接) */
  src: string;
  /** 数据现在的实际位置 */
  dst: string;
  /** 源目录备份;确认软件正常后删除,置 null */
  bak: string | null;
  bytes: number;
  fileCount: number;
  at: string;
}

export interface MigrateResult {
  movedBytes: number;
  fileCount: number;
  dst: string;
}

export type FileCategory = "video" | "archive" | "installer" | "image" | "other";

export interface BigFileInfo {
  path: string;
  name: string;
  sizeBytes: number;
  modifiedMs: number;
  category: FileCategory;
  deletable: boolean;
  /** 不可删时的白话解释 */
  reason: string | null;
}
