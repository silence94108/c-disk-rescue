<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  checkLocks,
  confirmMigration,
  getMigratables,
  getMigrateTargets,
  onMigrateProgress,
  requestClose,
  startMigrate,
  cancelMigrate,
} from "../api";
import type { MigratableItem, MigrateProgress, TargetDisk } from "../api/types";
import { scanSummary } from "../store";
import { fmtBytes } from "../utils/format";

const router = useRouter();

const items = ref<MigratableItem[]>([]);
const targets = ref<TargetDisk[]>([]);
const chosenTarget = ref<string>("");
const loading = ref(true);
const notice = ref("");

/* 向导状态机:idle=列表 / waiting=请退出软件 / moving=搬家中 / done=完成 */
const wizard = ref<"idle" | "waiting" | "moving" | "done">("idle");
const active = ref<MigratableItem | null>(null);
const lockedBy = ref<string[]>([]);
const closing = ref(false);
const progress = ref<MigrateProgress | null>(null);
const doneBytes = ref(0);
const confirming = ref(false);

let unlisten: UnlistenFn | null = null;
let lockTimer: number | undefined;

const percent = computed(() => {
  if (!progress.value || progress.value.totalBytes === 0) return 0;
  return Math.min(99, (progress.value.copiedBytes / progress.value.totalBytes) * 100);
});

onMounted(async () => {
  if (!scanSummary.value) {
    router.replace("/");
    return;
  }
  try {
    const [mig, tgs] = await Promise.all([getMigratables(), getMigrateTargets()]);
    items.value = mig;
    targets.value = tgs;
    chosenTarget.value = tgs.find((t) => t.recommended)?.mountPoint ?? "";
  } finally {
    loading.value = false;
  }
});

onUnmounted(() => {
  unlisten?.();
  if (lockTimer !== undefined) window.clearInterval(lockTimer);
});

function stopLockPolling() {
  if (lockTimer !== undefined) {
    window.clearInterval(lockTimer);
    lockTimer = undefined;
  }
}

/* 第一步:占用检查。有锁 → 停在「请退出」页轮询;无锁 → 直接开搬 */
async function begin(item: MigratableItem) {
  if (!chosenTarget.value) {
    notice.value = "没有找到能用的目标盘(需要一块 NTFS 格式的本地硬盘)";
    return;
  }
  notice.value = "";
  active.value = item;
  const st = await checkLocks([item.ruleId]).catch(() => []);
  lockedBy.value = st.find((s) => s.ruleId === item.ruleId)?.lockedBy ?? [];
  if (lockedBy.value.length === 0) {
    doMigrate();
    return;
  }
  wizard.value = "waiting";
  // 检测到进程退出后自动亮起下一步(设计规范 §3.4)
  lockTimer = window.setInterval(async () => {
    if (!active.value || wizard.value !== "waiting") return;
    const s = await checkLocks([active.value.ruleId]).catch(() => null);
    if (s) lockedBy.value = s.find((x) => x.ruleId === active.value!.ruleId)?.lockedBy ?? [];
  }, 2000);
}

/* 「帮我退出」:优雅关闭(等同点 ×),绝不强杀 */
async function helpClose() {
  if (!active.value || closing.value) return;
  closing.value = true;
  try {
    lockedBy.value = await requestClose(active.value.ruleId);
  } finally {
    closing.value = false;
  }
}

async function doMigrate() {
  if (!active.value) return;
  stopLockPolling();
  wizard.value = "moving";
  progress.value = null;
  try {
    unlisten = await onMigrateProgress((p) => {
      progress.value = p;
    });
    const result = await startMigrate(active.value.ruleId, chosenTarget.value);
    doneBytes.value = result.movedBytes;
    wizard.value = "done";
  } catch (e) {
    // 失败文案的重点是安抚:后端已自动回滚,数据无变化(设计规范 §3.4)
    notice.value = String(e);
    wizard.value = "idle";
    active.value = null;
  } finally {
    unlisten?.();
    unlisten = null;
  }
}

/* 完成页:确认软件正常 → 删除 .bak,C 盘空间此刻才真正腾出 */
async function confirmOk() {
  if (!active.value || confirming.value) return;
  confirming.value = true;
  try {
    await confirmMigration(active.value.ruleId);
    notice.value = `已腾出 ${fmtBytes(doneBytes.value)},${active.value.displayName}照常使用`;
    closeWizard(true);
  } catch (e) {
    notice.value = String(e);
  } finally {
    confirming.value = false;
  }
}

function later() {
  notice.value = "备份先留着,确认软件正常后到「已搬家」页删除备份腾空间";
  closeWizard(true);
}

function closeWizard(refresh: boolean) {
  stopLockPolling();
  wizard.value = "idle";
  const finished = active.value;
  active.value = null;
  if (refresh && finished) {
    items.value = items.value.filter((i) => i.ruleId !== finished.ruleId);
  }
}

function cancelMoving() {
  cancelMigrate();
}
</script>

<template>
  <main class="migrate">
    <header class="head">
      <button class="back" @click="router.push('/report')">‹ 返回报告</button>
      <div class="head-stat">
        <span class="head-title">搬家瘦身</span>
        <span class="head-sub">把大目录搬到其他盘,原位置留一个「传送门」,软件照常使用</span>
      </div>
      <button class="link moved-link" @click="router.push('/moved')">已搬家管理 ›</button>
    </header>

    <p v-if="notice" class="notice">{{ notice }}</p>

    <p v-if="loading" class="empty">正在整理…</p>
    <p v-else-if="items.length === 0" class="empty">
      没有找到可搬家的目录(微信、QQ 等装在本机才会出现)。
    </p>

    <template v-else>
      <!-- 目标盘选择:默认推荐剩余最大的 NTFS 本地盘 -->
      <section class="targets" v-if="targets.length > 0">
        <span class="targets-label">搬到:</span>
        <button
          v-for="t in targets"
          :key="t.mountPoint"
          class="target"
          :class="{ chosen: t.mountPoint === chosenTarget, bad: !t.isNtfs }"
          :disabled="!t.isNtfs"
          :title="t.isNtfs ? '' : '这个盘不是 NTFS 格式,搬过去软件会出问题'"
          @click="chosenTarget = t.mountPoint"
        >
          {{ t.mountPoint }}(剩 {{ fmtBytes(t.freeBytes) }})
          <em v-if="t.recommended">推荐</em>
          <em v-if="!t.isNtfs" class="bad-tag">不支持</em>
        </button>
      </section>
      <p v-else class="notice">
        没有找到能用的目标盘——需要一块除 C 盘外的本地硬盘(U 盘和移动硬盘不行,拔掉后软件会打不开)。
      </p>

      <section class="list">
        <div v-for="item in items" :key="item.ruleId" class="item">
          <div class="item-body">
            <div class="item-line1">
              <span class="item-name">{{ item.displayName }}</span>
              <span class="item-size num">{{ fmtBytes(item.sizeBytes) }}</span>
            </div>
            <p class="item-explain">{{ item.explain }}</p>
            <p class="item-dest" v-if="chosenTarget">
              将搬到 {{ chosenTarget }}\AppDataMove\ ·
              搬家后极小概率软件大版本更新出问题,出问题可一键搬回
            </p>
          </div>
          <button class="item-btn" :disabled="!chosenTarget" @click="begin(item)">
            开始搬家
          </button>
        </div>
      </section>
    </template>

    <!-- 向导浮层 -->
    <div class="mask" v-if="wizard !== 'idle' && active">
      <div class="dialog">
        <!-- 步骤1:请退出软件 -->
        <template v-if="wizard === 'waiting'">
          <p class="dlg-title">先退出{{ active.displayName.slice(0, 8) }}相关软件</p>
          <template v-if="lockedBy.length > 0">
            <p class="dlg-body">
              {{ lockedBy.join("、") }} 正在使用这些文件。请手动退出,或让我来:
            </p>
            <button class="primary-btn" :disabled="closing" @click="helpClose">
              {{ closing ? "正在请它退出…" : "帮我退出" }}
            </button>
            <p class="dlg-aux">退出后这里会自动亮起,不用刷新</p>
          </template>
          <template v-else>
            <p class="dlg-body ok">✓ 软件已退出,可以开始搬家了</p>
            <button class="primary-btn" @click="doMigrate">开始搬家</button>
          </template>
          <button class="link" @click="closeWizard(false)">先不搬了</button>
        </template>

        <!-- 步骤2:搬家进度 -->
        <template v-else-if="wizard === 'moving'">
          <p class="dlg-title">正在搬家…</p>
          <div class="bar">
            <div class="bar-fill" :style="{ width: percent + '%' }"></div>
          </div>
          <p class="dlg-body num" v-if="progress">
            正在搬 {{ fmtBytes(progress.totalBytes) }},已完成
            {{ fmtBytes(progress.copiedBytes) }}
          </p>
          <p class="dlg-body" v-else>正在准备…</p>
          <p class="dlg-aux">别关机哦(已自动帮你阻止电脑休眠)。源数据在搬家全程原样保留。</p>
          <button class="link" @click="cancelMoving">取消(不会有任何变化)</button>
        </template>

        <!-- 步骤3:完成,引导验证后删除备份 -->
        <template v-else-if="wizard === 'done'">
          <p class="dlg-title ok">搬家完成!</p>
          <p class="dlg-body">
            数据已在 {{ chosenTarget }} 盘安家。现在打开{{
              active.displayName.slice(0, 8)
            }}试试——一切正常的话,点下面按钮删除 C 盘备份,才会真正腾出
            {{ fmtBytes(doneBytes) }}。
          </p>
          <button class="primary-btn" :disabled="confirming" @click="confirmOk">
            {{ confirming ? "正在删除备份…" : "软件打开正常,删除备份腾空间" }}
          </button>
          <button class="link" @click="later">稍后再说(备份先留着)</button>
        </template>
      </div>
    </div>
  </main>
</template>

<style scoped>
.migrate {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 0 24px 24px;
  overflow-y: auto;
}

.head {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 16px 0;
}

.back {
  color: var(--color-primary);
  font-size: var(--font-size-card-title);
}

.head-stat {
  flex: 1;
  display: flex;
  flex-direction: column;
}

.head-title {
  font-size: var(--font-size-title);
  font-weight: 600;
}

.head-sub {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}

.moved-link {
  flex-shrink: 0;
}

.link {
  color: var(--color-primary);
  font-size: var(--font-size-body);
}

.notice {
  padding: 10px 14px;
  margin-bottom: 12px;
  border-radius: 8px;
  background: #dcfce7;
  color: var(--color-success);
}

.targets {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 16px;
}

.targets-label {
  color: var(--color-text-secondary);
}

.target {
  padding: 8px 14px;
  border-radius: 8px;
  border: 2px solid #e5e7eb;
  background: var(--color-card);
}

.target.chosen {
  border-color: var(--color-primary);
  color: var(--color-primary);
  font-weight: 600;
}

.target.bad {
  opacity: 0.55;
  cursor: not-allowed;
}

.target em {
  font-style: normal;
  font-size: var(--font-size-aux);
  color: var(--color-success);
  margin-left: 4px;
}

.target em.bad-tag {
  color: var(--color-text-secondary);
}

.list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.item {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 14px 16px;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
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
  margin: 2px 0;
}

.item-dest {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}

.item-btn {
  flex-shrink: 0;
  height: 40px;
  padding: 0 20px;
  border-radius: 8px;
  background: var(--color-primary);
  color: #fff;
  font-weight: 600;
}

.item-btn:disabled {
  background: #d1d5db;
}

.mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 10;
}

.dialog {
  width: 420px;
  padding: 28px 32px;
  background: var(--color-card);
  border-radius: var(--radius-card);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  text-align: center;
}

.dlg-title {
  font-size: var(--font-size-title);
  font-weight: 700;
}

.dlg-title.ok {
  color: var(--color-success);
}

.dlg-body {
  color: var(--color-text);
}

.dlg-body.ok {
  color: var(--color-success);
  font-weight: 600;
}

.dlg-aux {
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
}

.primary-btn:disabled {
  background: #93c5fd;
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

.empty {
  padding: 40px;
  text-align: center;
  color: var(--color-text-secondary);
}
</style>
