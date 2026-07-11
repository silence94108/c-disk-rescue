import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { DiskInfo, ScanProgress, ScanSummary, TreeNode } from "./types";

export const getDisks = () => invoke<DiskInfo[]>("get_disks");

export const startScan = (root?: string) =>
  invoke<ScanSummary>("start_scan", { root: root ?? null });

export const cancelScan = () => invoke<void>("cancel_scan");

export const getChildren = (nodeId: number) =>
  invoke<TreeNode[]>("get_children", { nodeId });

export const onScanProgress = (
  cb: (p: ScanProgress) => void,
): Promise<UnlistenFn> => listen<ScanProgress>("scan:progress", (e) => cb(e.payload));
