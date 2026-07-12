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

/** 自选搬家候选(知识库外的大文件夹,用户手动决定是否搬) */
export interface MigrateCandidate {
  id: string;
  name: string;
  path: string;
  displayName: string;
  sizeBytes: number;
  fileCount: number;
  /** safe 可搬 / cautious 可搬但谨慎 / blocked 不给搬 */
  status: "safe" | "cautious" | "blocked";
  note: string;
}

/** 自带官方「更改位置」入口的系统文件夹,出引导卡教官方搬法 */
export interface KnownFolderInfo {
  name: string;
  path: string;
  sizeBytes: number;
}

export interface MigrateCandidatesReport {
  candidates: MigrateCandidate[];
  knownFolders: KnownFolderInfo[];
}

/** 外部 junction:所有已搬走的目录(含非本工具搬的),指向别的盘、可搬回 */
export interface ExternalJunction {
  src: string;
  dst: string;
  name: string;
  sizeBytes: number;
  fileCount: number;
}

export interface CleanableItem {
  ruleId: string;
  displayName: string;
  explain: string;
  risk: Risk;
  needsAdmin: boolean;
  /** 引导型:工具不代删,explain 教手动操作;默认不勾且禁用勾选 */
  guideOnly: boolean;
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

/** 分段容量条的后端两段;临时·垃圾由前端合计,其他/剩余由前端差值推出 */
export interface CapacityBreakdown {
  systemBytes: number;
  appsBytes: number;
}

export interface OrphanProfile {
  /** 显示名(可能是乱码,前端原样呈现) */
  name: string;
  path: string;
  sizeBytes: number;
  fileCount: number;
  /** 来源软件线索(据 AppData 内部目录推断),如 ["腾讯电脑管家"] */
  hints: string[];
}

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
