<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  checkLocks,
  getMigratables,
  onCleanProgress,
  runClean,
  scanCleanables,
} from "../api";
import type {
  CleanableItem,
  CleanablesReport,
  CleanResult,
  MigratableItem,
} from "../api/types";
import { scanSummary, loadLastScan, saveLastScan } from "../store";
import { fmtBytes } from "../utils/format";

const router = useRouter();

const report = ref<CleanablesReport | null>(null);
const migratables = ref<MigratableItem[]>([]);
const phase = ref<"loading" | "ready" | "cleaning" | "done">("loading");
const showDetail = ref(false);
const checked = ref<Record<string, boolean>>({});
const progressBytes = ref(0);
const cleanResult = ref<CleanResult | null>(null);
const errorMsg = ref("");

let unlisten: UnlistenFn | null = null;

const items = computed(() => report.value?.items ?? []);
const junkTotal = computed(() => items.value.reduce((s, i) => s + i.sizeBytes, 0));
const migrateTotal = computed(() => migratables.value.reduce((s, i) => s + i.sizeBytes, 0));
/* 「预计可释放」只计垃圾 + 可搬家,大文件待用户决定不并入(设计规范 §3.2) */
const releasable = computed(() => junkTotal.value + migrateTotal.value);

function isDisabled(item: CleanableItem): boolean {
  if (item.lockedBy.length > 0) return true;
  if (item.needsAdmin && !report.value?.isElevated) return true;
  return false;
}

const selectedItems = computed(() =>
  items.value.filter((i) => checked.value[i.ruleId] && !isDisabled(i)),
);
const selectedBytes = computed(() =>
  selectedItems.value.reduce((s, i) => s + i.sizeBytes, 0),
);
const progressPercent = computed(() => {
  if (selectedBytes.value === 0) return 0;
  return Math.min(99, (progressBytes.value / selectedBytes.value) * 100);
});

interface Badge {
  icon: string;
  text: string;
  cls: string;
}

function badge(item: CleanableItem): Badge {
  if (item.lockedBy.length > 0)
    return {
      icon: "⚪",
      text: `请先完全退出 ${item.lockedBy.join("、")}(它可能还在后台运行)`,
      cls: "gray",
    };
  if (item.needsAdmin && !report.value?.isElevated)
    return { icon: "⚪", text: "需要管理员权限", cls: "gray" };
  if (item.risk === "safe") return { icon: "🟢", text: "放心删", cls: "safe" };
  if (item.risk === "cost") return { icon: "🟡", text: "删了有代价", cls: "cost" };
  return { icon: "🟠", text: "谨慎,想清楚再勾", cls: "caution" };
}

async function loadReport() {
  const [clean, mig] = await Promise.all([
    scanCleanables(),
    getMigratables().catch(() => [] as MigratableItem[]),
  ]);
  report.value = clean;
  migratables.value = mig;
  /* 默认勾选:放心删 + 有代价;谨慎级和被禁用项不勾(需求文档 F2 分级) */
  const next: Record<string, boolean> = {};
  for (const item of clean.items) {
    next[item.ruleId] = item.risk !== "caution" && !isDisabled(item);
  }
  checked.value = next;
}

onMounted(async () => {
  // 直接进入或刷新丢失扫描态时,退回首页重新体检
  if (!scanSummary.value) {
    router.replace("/");
    return;
  }
  try {
    await loadReport();
    phase.value = "ready";
  } catch {
    errorMsg.value = "整理清理项时出了点问题,回首页再体检一次就好";
    phase.value = "ready";
  }
  // 占用状态轮询:用户退出被检出的软件后,卡片自动解锁亮起,无需手动刷新。
  // 只在存在锁定项时才发起复查,轻量接口只重测文件锁、不重新统计大小。
  lockTimer = window.setInterval(async () => {
    if (phase.value !== "ready" || polling) return;
    const locked = items.value.filter((i) => i.lockedBy.length > 0);
    if (locked.length === 0) return;
    polling = true;
    try {
      const statuses = await checkLocks(locked.map((i) => i.ruleId));
      for (const st of statuses) {
        const item = items.value.find((i) => i.ruleId === st.ruleId);
        if (!item) continue;
        const wasLocked = item.lockedBy.length > 0;
        item.lockedBy = st.lockedBy;
        // 刚解锁的项按默认分级规则恢复勾选(锁定期间勾选框禁用,不存在用户选择被覆盖)
        if (wasLocked && st.lockedBy.length === 0 && !isDisabled(item)) {
          checked.value[item.ruleId] = item.risk !== "caution";
        }
      }
    } catch {
      /* 复查失败不打扰用户,下个周期再试 */
    } finally {
      polling = false;
    }
  }, 3000);
});

let lockTimer: number | undefined;
let polling = false;

onUnmounted(() => {
  unlisten?.();
  if (lockTimer !== undefined) window.clearInterval(lockTimer);
});

async function startClean() {
  if (phase.value !== "ready" || selectedItems.value.length === 0) return;
  errorMsg.value = "";
  phase.value = "cleaning";
  progressBytes.value = 0;
  try {
    unlisten = await onCleanProgress((p) => {
      progressBytes.value = p.freedBytes;
    });
    const result = await runClean(selectedItems.value.map((i) => i.ruleId));
    cleanResult.value = result;
    const last = loadLastScan();
    if (last) {
      saveLastScan({
        ...last,
        freedBytes: (last.freedBytes ?? 0) + result.freedBytes,
      });
    }
    phase.value = "done";
    // 后台刷新列表:已清理项归零消失,用户「再清一次」时数字是真的
    loadReport().catch(() => {});
  } catch {
    errorMsg.value = "清理没有完成,已删除的部分是安全的,再试一次就好";
    phase.value = "ready";
  } finally {
    unlisten?.();
    unlisten = null;
  }
}
</script>

<template>
  <main class="report">
    <header class="head">
      <button class="back" @click="router.push('/')">‹ 首页</button>
      <span class="head-title">体检报告</span>
    </header>

    <p v-if="phase === 'loading'" class="empty">正在整理清理项…</p>

    <template v-else>
      <!-- 完成态:结果卡置顶,数字说话形成闭环(设计规范 §6) -->
      <section v-if="phase === 'done' && cleanResult" class="result-card">
        <p class="result-hero num">释放了 {{ fmtBytes(cleanResult.freedBytes) }}!</p>
        <p class="result-sub num">
          共删除 {{ cleanResult.deletedFiles }} 个文件<template
            v-if="cleanResult.failedFiles > 0"
          >,另有 {{ cleanResult.failedFiles }} 个文件正被使用,已自动跳过</template
          >
        </p>
        <p v-for="s in cleanResult.skipped" :key="s.ruleId" class="result-skip">
          「{{ items.find((i) => i.ruleId === s.ruleId)?.displayName ?? s.ruleId }}」未清理:{{
            s.reason
          }}
        </p>
        <button class="primary-btn" @click="router.push('/')">回首页看看效果</button>
      </section>

      <template v-else>
        <h1 class="hero num">体检完成!预计可释放 {{ fmtBytes(releasable) }}</h1>

        <section class="cards">
          <div class="card active">
            <span class="card-name">垃圾文件</span>
            <span class="card-size num">{{ fmtBytes(junkTotal) }}</span>
            <span class="card-note">可一键清理</span>
          </div>
          <div class="card">
            <span class="card-name">可搬家</span>
            <span class="card-size num">{{ fmtBytes(migrateTotal) }}</span>
            <span class="card-note">搬家功能下个版本开放</span>
          </div>
          <div class="card">
            <span class="card-name">大文件</span>
            <span class="card-size">—</span>
            <span class="card-note">即将上线,待你决定</span>
          </div>
        </section>

        <!-- 清理中:进度可感知(>300ms 必须有反馈,设计规范 §6) -->
        <section v-if="phase === 'cleaning'" class="cleaning">
          <div class="bar">
            <div class="bar-fill" :style="{ width: progressPercent + '%' }"></div>
          </div>
          <p class="cleaning-stat num">已释放 {{ fmtBytes(progressBytes) }}…</p>
        </section>

        <template v-else>
          <button
            class="primary-btn"
            :disabled="selectedItems.length === 0"
            @click="startClean"
          >
            一键优化(清理选中的 {{ fmtBytes(selectedBytes) }})
          </button>
          <div class="links">
            <button class="link" @click="showDetail = !showDetail">
              {{ showDetail ? "收起明细 ‹" : "查看明细,自己决定 ›" }}
            </button>
            <button class="link" @click="router.push('/map')">查看空间分布 ›</button>
          </div>
        </template>

        <p class="error" v-if="errorMsg">{{ errorMsg }}</p>

        <!-- Tab1 清理明细:勾选框 + 白话名 + 大小 + 后果说明 四要素(设计规范 §3.3) -->
        <section class="list" v-if="showDetail">
          <p v-if="items.length === 0" class="empty">
            没有找到可清理的垃圾,你的 C 盘很干净。
          </p>
          <label
            v-for="item in items"
            :key="item.ruleId"
            class="item"
            :class="{ disabled: isDisabled(item) }"
            :title="item.path"
          >
            <input
              type="checkbox"
              v-model="checked[item.ruleId]"
              :disabled="isDisabled(item) || phase === 'cleaning'"
            />
            <div class="item-body">
              <div class="item-line1">
                <span class="item-name">{{ item.displayName }}</span>
                <span class="item-size num">{{ fmtBytes(item.sizeBytes) }}</span>
              </div>
              <p class="item-explain">{{ item.explain }}</p>
              <span class="item-badge" :class="badge(item).cls">
                {{ badge(item).icon }} {{ badge(item).text }}
              </span>
            </div>
          </label>
        </section>
      </template>
    </template>
  </main>
</template>

<style scoped>
.report {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 0 24px 24px;
  overflow-y: auto;
}

.head {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 16px 0;
}

.back {
  color: var(--color-primary);
  font-size: var(--font-size-card-title);
}

.head-title {
  font-size: var(--font-size-title);
  font-weight: 600;
}

.hero {
  font-size: var(--font-size-title);
  font-weight: 700;
  margin: 8px 0 20px;
}

.cards {
  display: flex;
  gap: 12px;
  margin-bottom: 24px;
}

.card {
  width: 180px;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
  border: 2px solid transparent;
}

.card.active {
  border-color: var(--color-primary);
}

.card-name {
  font-size: var(--font-size-body);
  color: var(--color-text-secondary);
}

.card-size {
  font-size: 24px;
  font-weight: 700;
}

.card-note {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}

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

.primary-btn:hover:not(:disabled) {
  filter: brightness(1.08);
}

.primary-btn:disabled {
  background: #d1d5db;
  cursor: not-allowed;
}

.links {
  display: flex;
  gap: 24px;
  margin-top: 12px;
}

.link {
  color: var(--color-primary);
  font-size: var(--font-size-body);
}

.cleaning {
  width: 100%;
  max-width: 480px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  margin: 8px 0;
}

.bar {
  width: 100%;
  height: 10px;
  border-radius: 5px;
  background: #e5e7eb;
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  border-radius: 5px;
  background: var(--color-primary);
  transition: width 0.2s;
}

.cleaning-stat {
  font-size: var(--font-size-card-title);
}

.error {
  margin-top: 12px;
  color: var(--color-danger);
}

.list {
  width: 100%;
  max-width: 640px;
  margin-top: 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.item {
  display: flex;
  gap: 12px;
  padding: 14px 16px;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
  cursor: pointer;
}

.item.disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.item input {
  margin-top: 4px;
  width: 16px;
  height: 16px;
  accent-color: var(--color-primary);
  flex-shrink: 0;
}

.item-body {
  flex: 1;
  min-width: 0;
}

.item-line1 {
  display: flex;
  justify-content: space-between;
  gap: 12px;
}

.item-name {
  font-size: var(--font-size-card-title);
  font-weight: 600;
}

.item-size {
  font-weight: 700;
}

.item-explain {
  color: var(--color-text-secondary);
  margin: 2px 0 6px;
}

.item-badge {
  font-size: var(--font-size-aux);
}

.item-badge.safe {
  color: var(--color-success);
}

.item-badge.cost {
  color: var(--color-warning);
}

.item-badge.caution {
  color: #ea580c;
}

.item-badge.gray {
  color: var(--color-text-secondary);
}

.result-card {
  margin-top: 32px;
  padding: 32px 48px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
}

.result-hero {
  font-size: var(--font-size-hero);
  font-weight: 700;
  color: var(--color-success);
}

.result-sub {
  color: var(--color-text-secondary);
}

.result-skip {
  font-size: var(--font-size-aux);
  color: var(--color-warning);
}

.result-card .primary-btn {
  margin-top: 16px;
}

.empty {
  padding: 40px;
  text-align: center;
  color: var(--color-text-secondary);
}
</style>
