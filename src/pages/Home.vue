<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import DonutGauge from "../components/DonutGauge.vue";
import { cancelScan, getDisks } from "../api";
import type { DiskInfo } from "../api/types";
import {
  breakdown,
  cleanPhase,
  cleanProgressBytes,
  cleanReport,
  junkTotal,
  loadLastScan,
  loadReportData,
  migrateTotal,
  recoverNotice,
  releasable,
  runScanFlow,
  scanning,
  scanProgress,
  scanSummary,
  selectedBytes,
  selectedItems,
  startCleanSelected,
  type LastScan,
} from "../store";
import { fmtBytes, fmtCount, fmtRelativeTime } from "../utils/format";

const router = useRouter();

const disk = ref<DiskInfo | null>(null);
const lastScan = ref<LastScan | null>(loadLastScan());
const errorMsg = ref("");
const doneMsg = ref("");

const phase = computed<"idle" | "scanning" | "ready">(() => {
  if (scanning.value) return "scanning";
  return scanSummary.value ? "ready" : "idle";
});

const usedPercent = computed(() => {
  if (!disk.value || disk.value.totalBytes === 0) return 0;
  return ((disk.value.totalBytes - disk.value.freeBytes) / disk.value.totalBytes) * 100;
});

const usedBytes = computed(() =>
  disk.value ? disk.value.totalBytes - disk.value.freeBytes : 0,
);

/* 环色:>90% 危险红 + 呼吸;其余渐变蓝(F 稿 85% 即蓝渐变,警示语义由副标题承担) */
const danger = computed(() => usedPercent.value > 90);

const healthText = computed(() => {
  if (usedPercent.value > 90) return "空间快满了,建议马上清理";
  if (usedPercent.value > 80) return "已用超过八成,建议清理";
  return "空间还算充裕";
});

/* 分段容量条:后端两段 + 前端垃圾合计,其他段吸收全部误差;
   breakdown 取数失败降级为「已用/剩余」两段,不显示假数据(设计规范 §3.1) */
interface Seg {
  name: string;
  bytes: number;
  color: string;
}

const segs = computed<Seg[]>(() => {
  const d = disk.value;
  if (!d || d.totalBytes === 0) return [];
  const bd = breakdown.value;
  if (!bd || phase.value !== "ready") {
    return [
      { name: "已用", bytes: usedBytes.value, color: "var(--seg-apps)" },
      { name: "剩余", bytes: d.freeBytes, color: "var(--seg-free)" },
    ];
  }
  const junk = junkTotal.value;
  const other = Math.max(
    0,
    usedBytes.value - bd.systemBytes - bd.appsBytes - junk,
  );
  return [
    { name: "系统与保留", bytes: bd.systemBytes, color: "var(--seg-system)" },
    { name: "已装应用", bytes: bd.appsBytes, color: "var(--seg-apps)" },
    { name: "临时/垃圾", bytes: junk, color: "var(--seg-junk)" },
    { name: "其他", bytes: other, color: "var(--seg-other)" },
    { name: "剩余", bytes: d.freeBytes, color: "var(--seg-free)" },
  ];
});

function segWidth(s: Seg): string {
  const total = disk.value?.totalBytes ?? 0;
  if (total === 0) return "0%";
  return `${(s.bytes / total) * 100}%`;
}

async function refreshDisk() {
  try {
    const disks = await getDisks();
    disk.value = disks.find((d) => d.isSystem) ?? disks[0] ?? null;
  } catch {
    errorMsg.value = "没读到磁盘信息,重启软件试试";
  }
}

onMounted(() => {
  refreshDisk();
  // 体检过但报告数据缺失(如加载失败过)时补拉
  if (scanSummary.value && !cleanReport.value) {
    loadReportData().catch(() => {});
  }
});

async function startCheck() {
  errorMsg.value = "";
  doneMsg.value = "";
  try {
    await runScanFlow();
    lastScan.value = loadLastScan();
  } catch (e) {
    if (String(e) !== "cancelled") {
      errorMsg.value = "扫描没有完成,你的电脑没有任何变化,再试一次就好";
    }
  }
}

async function optimize() {
  if (selectedItems.value.length === 0) return;
  errorMsg.value = "";
  doneMsg.value = "";
  try {
    const result = await startCleanSelected();
    if (result) {
      doneMsg.value = `释放了 ${fmtBytes(result.freedBytes)}!共删除 ${result.deletedFiles} 个文件`;
      lastScan.value = loadLastScan();
      refreshDisk();
    }
  } catch {
    errorMsg.value = "清理没有完成,已删除的部分是安全的,再试一次就好";
  }
}

const cleanPercent = computed(() => {
  if (selectedBytes.value === 0) return 0;
  return Math.min(99, (cleanProgressBytes.value / selectedBytes.value) * 100);
});
</script>

<template>
  <div class="page">
    <p class="recover" v-if="recoverNotice">{{ recoverNotice }}</p>

    <!-- 容量大卡(F 稿 .ov):环形 + 分段条 + 图例 -->
    <section class="ov">
      <svg class="art" viewBox="0 0 200 150" aria-hidden="true">
        <ellipse cx="100" cy="128" rx="58" ry="12" fill="#cfe1f9" opacity=".55"/>
        <path d="M42 76 L100 105 L100 126 L42 97 Z" fill="#b7d2f8"/>
        <path d="M158 76 L100 105 L100 126 L158 97 Z" fill="#9cc0f4"/>
        <path d="M42 76 L100 47 L158 76 L100 105 Z" fill="#e7f0fe"/>
        <ellipse cx="100" cy="76" rx="33" ry="16.5" fill="#fff"/>
        <ellipse cx="100" cy="76" rx="19" ry="9.5" fill="#dcebfe"/>
        <ellipse cx="100" cy="76" rx="8" ry="4.2" fill="#2563eb"/>
        <text x="100" y="78.6" text-anchor="middle" font-size="6.5" font-weight="700" fill="#fff">C</text>
        <path d="M52 92 l11 5.6 v5.4 l-11 -5.6 Z" fill="#2f7cf0"/>
        <path d="M67 99.5 l8 4 v5 l-8 -4 Z" fill="#7db4ff"/>
        <g transform="translate(148,20)"><path d="M0 7 L12 0 L24 7 L12 14 Z" fill="#e7f0fe"/><path d="M0 7 L12 14 V26 L0 19 Z" fill="#b7d2f8"/><path d="M24 7 L12 14 V26 L24 19 Z" fill="#9cc0f4"/></g>
        <circle cx="30" cy="52" r="4" fill="#9cc4fa"/>
        <circle cx="172" cy="52" r="3" fill="#bcd7fb"/>
        <circle cx="166" cy="112" r="4.5" fill="#cfe3fc"/>
        <circle cx="22" cy="96" r="2.5" fill="#cfe3fc"/>
      </svg>
      <h3>C 盘 (系统盘)</h3>
      <div class="sub" v-if="disk">
        容量 {{ fmtBytes(disk.totalBytes) }} · {{ healthText }}
      </div>
      <div class="sub" v-else>正在读取磁盘信息…</div>

      <div class="ovrow">
        <!-- 扫描中环形转进度环(设计规范 §3.1) -->
        <div class="scan-ring" v-if="phase === 'scanning'">
          <div class="ring-spin"></div>
          <div class="ring-ct">
            <template v-if="scanProgress">
              <span class="ring-num num">{{ fmtCount(scanProgress.scannedFiles) }}</span>
              <span class="ring-sub">个文件</span>
            </template>
            <span class="ring-sub" v-else>正在准备…</span>
          </div>
        </div>
        <DonutGauge
          v-else
          :percent="usedPercent"
          :size="150"
          :stroke-width="11"
          :color="'var(--color-danger)'"
          :gradient="danger ? undefined : ['#7db4ff', '#2563eb']"
          :breathing="danger"
        >
          <span class="pct num" :class="{ bad: danger }"
            >{{ usedPercent.toFixed(0) }}<small>%</small></span
          >
        </DonutGauge>

        <div class="usage" v-if="disk">
          <div class="uline">
            已用 <b class="num">{{ fmtBytes(usedBytes) }}</b> / {{ fmtBytes(disk.totalBytes) }} ·
            剩余 <b class="num">{{ fmtBytes(disk.freeBytes) }}</b>
          </div>
          <div class="segs">
            <i
              v-for="s in segs"
              :key="s.name"
              :style="{ width: segWidth(s), background: s.color }"
            ></i>
          </div>
          <div class="legend">
            <div class="lg" v-for="s in segs" :key="s.name">
              <span class="t"><i :style="{ background: s.color }"></i>{{ s.name }}</span>
              <span class="v num">{{ fmtBytes(s.bytes) }}</span>
            </div>
          </div>
          <button class="map-link" v-if="phase === 'ready'" @click="router.push('/map')">
            查看空间分布 ›
          </button>
        </div>
      </div>
    </section>

    <p class="done" v-if="doneMsg">{{ doneMsg }}</p>
    <p class="error" v-if="errorMsg">{{ errorMsg }}</p>

    <!-- 绿色行动区(F 稿 .cta):唯一主按钮所在 -->
    <section class="cta">
      <span class="tile"><svg class="ic"><use href="#i-broom" /></svg></span>

      <template v-if="phase === 'idle'">
        <div class="tx">
          <b>先给 C 盘做个体检</b>
          <p>找出能安全腾出的空间,体检后一键清理垃圾与搬家瘦身</p>
        </div>
        <button class="btn" @click="startCheck">开始体检（约 1 分钟）</button>
      </template>

      <template v-else-if="phase === 'scanning'">
        <div class="tx">
          <b>正在体检…<span class="num" v-if="scanProgress">已扫描 {{ fmtBytes(scanProgress.scannedBytes) }}</span></b>
          <p class="scan-path">{{ scanProgress?.currentPath ?? "正在准备扫描…" }}</p>
        </div>
        <button class="ghost" @click="cancelScan()">取消</button>
      </template>

      <template v-else-if="cleanPhase === 'cleaning'">
        <div class="tx">
          <b>正在清理…</b>
          <div class="bar">
            <div class="bar-fill" :style="{ width: cleanPercent + '%' }"></div>
          </div>
          <p class="num">已释放 {{ fmtBytes(cleanProgressBytes) }}</p>
        </div>
      </template>

      <template v-else>
        <div class="tx">
          <b>可清理约 <i class="num">{{ fmtBytes(releasable) }}</i></b>
          <p>
            垃圾 {{ fmtBytes(junkTotal) }} · 可搬家 {{ fmtBytes(migrateTotal) }};
            想自己挑,去左侧「垃圾清理」看明细
          </p>
        </div>
        <button class="btn" :disabled="selectedItems.length === 0" @click="optimize">
          {{
            selectedItems.length > 0
              ? `一键优化（清理选中的 ${fmtBytes(selectedBytes)}）`
              : "没有可自动清理的项"
          }}
        </button>
      </template>
    </section>

    <!-- 上次体检条(F 稿 .lastrow):建立信任 -->
    <div class="lastrow" v-if="lastScan && phase !== 'scanning'">
      <svg class="ic"><use href="#i-clock" /></svg>
      <span v-if="lastScan.freedBytes"
        >上次体检 · {{ fmtRelativeTime(lastScan.at) }} · 释放了
        <span class="num">{{ fmtBytes(lastScan.freedBytes) }}</span></span
      >
      <span v-else
        >上次体检 · {{ fmtRelativeTime(lastScan.at) }} · C盘共占用
        <span class="num">{{ fmtBytes(lastScan.totalBytes) }}</span></span
      >
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.recover {
  padding: 10px 16px;
  border-radius: 10px;
  background: #e2f6ec;
  color: var(--pill-safe-fg);
}

.done {
  padding: 10px 16px;
  border-radius: 10px;
  background: #e2f6ec;
  color: var(--pill-safe-fg);
  font-weight: 600;
}

.error {
  padding: 10px 16px;
  border-radius: 10px;
  background: #fdeaea;
  color: var(--color-danger);
}

/* 容量大卡 */
.ov {
  background: var(--color-card);
  border-radius: var(--radius-card);
  padding: 26px 28px;
  position: relative;
  box-shadow: var(--shadow-card);
}

.ov h3 {
  font-size: 26px;
  font-weight: 900;
  color: var(--color-text);
}

.ov .sub {
  font-size: 14px;
  color: var(--color-text-secondary);
  margin-top: 4px;
}

.art {
  position: absolute;
  top: 14px;
  right: 20px;
  width: 132px;
  pointer-events: none;
}

.ovrow {
  display: flex;
  align-items: center;
  gap: 28px;
  margin-top: 24px;
}

.pct {
  font-size: 36px;
  font-weight: 900;
  color: var(--color-primary);
}

.pct small {
  font-size: 16px;
  font-weight: 700;
}

.pct.bad {
  color: var(--color-danger);
}

/* 扫描中的转圈环(占位 150px,与容量环同位) */
.scan-ring {
  position: relative;
  width: 150px;
  height: 150px;
  flex-shrink: 0;
}

.ring-spin {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  border: 11px solid #e8eefb;
  border-top-color: var(--color-primary);
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.ring-ct {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.ring-num {
  font-size: 24px;
  font-weight: 900;
}

.ring-sub {
  font-size: 13px;
  color: var(--color-text-secondary);
}

.usage {
  flex: 1;
  min-width: 0;
}

.uline {
  font-size: 15px;
  color: var(--color-text-secondary);
}

.uline b {
  color: var(--color-text);
  font-weight: 900;
  font-size: 16.5px;
}

.segs {
  display: flex;
  gap: 3px;
  height: 18px;
  border-radius: 10px;
  overflow: hidden;
  margin: 14px 0 16px;
  background: #eef2f8;
}

.segs i {
  display: block;
  height: 100%;
  transition: width 0.6s ease;
}

.legend {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
}

.lg {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.lg .t {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 13px;
  color: var(--color-text);
  font-weight: 600;
}

.lg .t i {
  width: 10px;
  height: 10px;
  border-radius: 3px;
  flex-shrink: 0;
}

.lg .v {
  font-size: 13px;
  color: var(--color-text-secondary);
  padding-left: 17px;
}

.map-link {
  margin-top: 12px;
  color: var(--color-primary);
  font-size: 14px;
  padding: 0;
}

/* 行动区 */
.cta {
  background: var(--color-card);
  border-radius: var(--radius-card);
  padding: 18px 20px;
  display: flex;
  align-items: center;
  gap: 16px;
  box-shadow: 0 14px 34px -22px rgba(31, 66, 135, 0.2);
}

.tile {
  width: 56px;
  height: 56px;
  border-radius: 16px;
  background: #e2f6ec;
  color: var(--color-action);
  display: grid;
  place-items: center;
  font-size: 26px;
  flex-shrink: 0;
}

.tx {
  flex: 1;
  min-width: 0;
}

.tx b {
  font-size: 19px;
  font-weight: 900;
  color: var(--color-text);
}

.tx b i {
  font-style: normal;
  color: var(--color-action);
}

.tx b span {
  font-size: 14px;
  font-weight: 500;
  color: var(--color-text-secondary);
  margin-left: 10px;
}

.tx p {
  font-size: 13.5px;
  color: var(--color-text-secondary);
  margin-top: 2px;
}

.scan-path {
  max-width: 480px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.btn {
  height: 52px;
  padding: 0 30px;
  border-radius: 12px;
  background: var(--color-action);
  color: #fff;
  font-size: 16.5px;
  font-weight: 800;
  transition: background 0.16s;
  white-space: nowrap;
}

.btn:hover:not(:disabled) {
  background: var(--color-action-deep);
}

.btn:disabled {
  background: #d1d5db;
  cursor: not-allowed;
}

.ghost {
  height: 44px;
  padding: 0 24px;
  border-radius: 10px;
  border: 1px solid var(--color-line);
  background: var(--color-card);
  color: var(--color-text-secondary);
}

.bar {
  width: 100%;
  max-width: 420px;
  height: 10px;
  border-radius: 5px;
  background: #e8eefb;
  overflow: hidden;
  margin: 8px 0 4px;
}

.bar-fill {
  height: 100%;
  border-radius: 5px;
  background: var(--color-action);
  transition: width 0.2s;
}

/* 上次体检条 */
.lastrow {
  background: rgba(255, 255, 255, 0.72);
  border: 1px solid #e6edf7;
  border-radius: 14px;
  padding: 13px 18px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
  color: var(--color-text-secondary);
}
</style>
