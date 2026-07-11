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
