<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import DonutGauge from "../components/DonutGauge.vue";
import { getDisks, startScan, cancelScan, onScanProgress } from "../api";
import type { DiskInfo, ScanProgress } from "../api/types";
import { scanSummary, loadLastScan, saveLastScan, type LastScan } from "../store";
import { fmtBytes, fmtCount, fmtRelativeTime } from "../utils/format";

const router = useRouter();

const disk = ref<DiskInfo | null>(null);
const phase = ref<"idle" | "scanning">("idle");
const progress = ref<ScanProgress | null>(null);
const lastScan = ref<LastScan | null>(loadLastScan());
const errorMsg = ref("");

let unlisten: UnlistenFn | null = null;

const usedPercent = computed(() => {
  if (!disk.value) return 0;
  return ((disk.value.totalBytes - disk.value.freeBytes) / disk.value.totalBytes) * 100;
});

/* 色彩阈值:>90% 危险爆红,80~90% 警示(设计规范 §4.1) */
const ringColor = computed(() => {
  if (usedPercent.value > 90) return "var(--color-danger)";
  if (usedPercent.value > 80) return "var(--color-warning)";
  return "var(--color-primary)";
});

onMounted(async () => {
  try {
    const disks = await getDisks();
    disk.value = disks.find((d) => d.isSystem) ?? disks[0] ?? null;
  } catch (e) {
    errorMsg.value = "没读到磁盘信息,重启软件试试";
  }
});

onUnmounted(() => {
  unlisten?.();
});

async function startCheck() {
  if (phase.value === "scanning") return;
  errorMsg.value = "";
  phase.value = "scanning";
  progress.value = null;
  try {
    unlisten = await onScanProgress((p) => {
      progress.value = p;
    });
    const summary = await startScan();
    scanSummary.value = summary;
    saveLastScan({
      at: Date.now(),
      totalBytes: summary.totalBytes,
      freedBytes: lastScan.value?.freedBytes,
    });
    router.push("/report");
  } catch (e) {
    if (String(e) !== "cancelled") {
      errorMsg.value = "扫描没有完成,你的电脑没有任何变化,再试一次就好";
    }
    phase.value = "idle";
  } finally {
    unlisten?.();
    unlisten = null;
  }
}

function cancel() {
  cancelScan();
}
</script>

<template>
  <main class="home">
    <h1 class="title">C盘救星</h1>

    <template v-if="phase === 'idle'">
      <DonutGauge
        :percent="usedPercent"
        :size="200"
        :color="ringColor"
        :breathing="usedPercent > 90"
      >
        <span class="pct num">{{ usedPercent.toFixed(0) }}%</span>
        <span class="free" v-if="disk">剩余 {{ fmtBytes(disk.freeBytes) }}</span>
      </DonutGauge>

      <button class="primary-btn" @click="startCheck">开始体检(约1分钟)</button>

      <p class="last-scan" v-if="lastScan">
        <template v-if="lastScan.freedBytes"
          >上次体检:释放了 {{ fmtBytes(lastScan.freedBytes) }} ·
          {{ fmtRelativeTime(lastScan.at) }}</template
        >
        <template v-else
          >上次体检:C盘共占用 {{ fmtBytes(lastScan.totalBytes) }} ·
          {{ fmtRelativeTime(lastScan.at) }}</template
        >
      </p>
      <p class="error" v-if="errorMsg">{{ errorMsg }}</p>
    </template>

    <template v-else>
      <div class="spinner" aria-label="正在扫描"></div>
      <p class="scan-stat num" v-if="progress">
        已扫描 {{ fmtCount(progress.scannedFiles) }} 个文件 · {{ fmtBytes(progress.scannedBytes) }}
      </p>
      <p class="scan-stat" v-else>正在准备扫描…</p>
      <p class="scan-path" v-if="progress">{{ progress.currentPath }}</p>
      <button class="ghost-btn" @click="cancel">取消</button>
    </template>
  </main>
</template>

<style scoped>
.home {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 24px;
  padding: 24px;
}

.title {
  font-size: var(--font-size-title);
  font-weight: 600;
}

.pct {
  font-size: var(--font-size-hero);
  font-weight: 700;
}

.free {
  font-size: var(--font-size-body);
  color: var(--color-text-secondary);
}

/* 主按钮:高 48px、≥240px,一屏只允许一个(设计规范 §4.3) */
.primary-btn {
  height: 48px;
  min-width: 240px;
  padding: 0 32px;
  border-radius: 10px;
  background: var(--color-primary);
  color: #fff;
  font-size: var(--font-size-card-title);
  font-weight: 600;
  transition: filter 0.15s;
}

.primary-btn:hover {
  filter: brightness(1.08);
}

.last-scan {
  color: var(--color-text-secondary);
  font-size: var(--font-size-aux);
}

.error {
  color: var(--color-danger);
}

.spinner {
  width: 72px;
  height: 72px;
  border-radius: 50%;
  border: 8px solid #e5e7eb;
  border-top-color: var(--color-primary);
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.scan-stat {
  font-size: var(--font-size-card-title);
}

.scan-path {
  max-width: 620px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--color-text-secondary);
  font-size: var(--font-size-aux);
}

.ghost-btn {
  padding: 8px 24px;
  border-radius: 8px;
  color: var(--color-text-secondary);
  border: 1px solid #e5e7eb;
  background: var(--color-card);
}
</style>
