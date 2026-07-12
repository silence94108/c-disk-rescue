<script setup lang="ts">
import { computed, onMounted, onUnmounted, provide, reactive, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import TreeRow from "../components/TreeRow.vue";
import {
  confirmMigration,
  evaluateMigratePick,
  getChildren,
  getMigrateTargets,
  onMigrateProgress,
  startMigrate,
} from "../api";
import type { MigratePickEval, MigrateProgress, TargetDisk, TreeNode } from "../api/types";
import { isStale, runScanFlow, scanning, scanProgress, scanSummary } from "../store";
import { fmtBytes, fmtCount } from "../utils/format";

const router = useRouter();
const roots = ref<TreeNode[]>([]);
const loading = ref(true);
const errorMsg = ref("");

const summary = computed(() => scanSummary.value);
const rootMax = computed(() => roots.value.reduce((m, c) => Math.max(m, c.sizeBytes), 0));

/* 空间分布依赖整棵扫描树(未持久化):缓存态(重启后 Rust 树为空)需重新体检 */
async function load() {
  if (!summary.value) return;
  loading.value = true;
  errorMsg.value = "";
  try {
    roots.value = await getChildren(summary.value.rootId);
  } catch {
    errorMsg.value = "stale";
  } finally {
    loading.value = false;
  }
}

async function rescan() {
  try {
    await runScanFlow();
    await load();
  } catch {
    /* 扫描失败由概览页兜底,这里静默 */
  }
}

// ─── 自定义转移(空间分布自选)──────────────────────────────
/* 本次会话内已搬走的节点 id:TreeRow inject 后就地标「已搬走」并隐藏转移按钮 */
const movedIds = reactive(new Set<number>());
const targets = ref<TargetDisk[]>([]);
const chosenTarget = ref<string>("");

/* 向导:closed / evaluating=后端评估中 / confirm=风险确认(含 blocked)/ moving / done */
const pickState = ref<"closed" | "evaluating" | "confirm" | "moving" | "done">("closed");
const evalResult = ref<MigratePickEval | null>(null);
const pickNodeId = ref<number>(-1);
const progress = ref<MigrateProgress | null>(null);
const doneBytes = ref(0);
const pickNotice = ref("");
const confirming = ref(false);
let unlisten: UnlistenFn | null = null;

const noTarget = computed(() => targets.value.length === 0 || !chosenTarget.value);
const percent = computed(() => {
  if (!progress.value || progress.value.totalBytes === 0) return 0;
  return Math.min(99, (progress.value.copiedBytes / progress.value.totalBytes) * 100);
});

/* 点某目录的「转移」:后端评估(还原路径 + 硬拦系统区 + 风险评级),过审才发 pickId */
async function onPickMigrate(node: TreeNode) {
  pickNotice.value = "";
  pickNodeId.value = node.id;
  evalResult.value = null;
  pickState.value = "evaluating";
  try {
    evalResult.value = await evaluateMigratePick(node.id);
  } catch (e) {
    pickNotice.value = String(e);
  } finally {
    pickState.value = "confirm";
  }
}

async function doMigrate() {
  const ev = evalResult.value;
  if (!ev?.pickId || noTarget.value) return;
  const finishedId = pickNodeId.value;
  pickState.value = "moving";
  progress.value = null;
  pickNotice.value = "";
  try {
    unlisten = await onMigrateProgress((p) => {
      progress.value = p;
    });
    const result = await startMigrate(ev.pickId, chosenTarget.value);
    doneBytes.value = result.movedBytes;
    movedIds.add(finishedId);
    pickState.value = "done";
  } catch (e) {
    // 失败后端已自动回滚,数据无变化;退回确认页给出提示
    pickNotice.value = String(e);
    pickState.value = "confirm";
  } finally {
    unlisten?.();
    unlisten = null;
  }
}

/* 完成页:确认软件正常 → 删 .bak,C 盘此刻才真正腾出(与搬家页一致) */
async function confirmOk() {
  const ev = evalResult.value;
  if (!ev?.pickId || confirming.value) return;
  confirming.value = true;
  try {
    await confirmMigration(ev.pickId);
    closePick();
  } catch (e) {
    pickNotice.value = String(e);
  } finally {
    confirming.value = false;
  }
}

function closePick() {
  pickState.value = "closed";
  evalResult.value = null;
  pickNodeId.value = -1;
  progress.value = null;
  pickNotice.value = "";
}

async function openFolder(path: string) {
  try {
    await revealItemInDir(path);
  } catch {
    /* 忽略打开失败 */
  }
}

provide("pickMigrate", onPickMigrate);
provide("movedIds", movedIds);

onMounted(async () => {
  // 缓存态(有上次结果但无 live 树)不直接请求,展示重新体检闸
  if (summary.value && !isStale.value) {
    load();
  } else {
    loading.value = false;
  }
  // 目标盘实时现取(缓存态也能用)
  try {
    const tgs = await getMigrateTargets();
    targets.value = tgs;
    chosenTarget.value = tgs.find((t) => t.recommended)?.mountPoint ?? "";
  } catch {
    /* 无目标盘时转移弹窗会提示 */
  }
});

onUnmounted(() => {
  unlisten?.();
});
</script>

<template>
  <main class="map">
    <header class="head">
      <button class="back" @click="router.push('/')">‹ 概览</button>
      <div class="head-stat">
        <span class="head-title">C盘空间分布</span>
        <span class="head-sub num" v-if="summary">
          共 {{ fmtBytes(summary.totalBytes) }} · 扫描用时
          {{ (summary.elapsedMs / 1000).toFixed(1) }} 秒
        </span>
      </div>
    </header>

    <!-- 缓存态 / 没有 live 树:需要重新体检才能看空间分布 -->
    <section class="gate" v-if="isStale || !summary || errorMsg === 'stale'">
      <template v-if="scanning">
        <div class="gate-spin"></div>
        <p class="gate-title num" v-if="scanProgress">
          正在体检 · 已扫描 {{ fmtCount(scanProgress.scannedFiles) }} 个文件
        </p>
        <p class="gate-title" v-else>正在体检…</p>
        <p class="gate-sub">{{ scanProgress?.currentPath ?? "正在准备…" }}</p>
      </template>
      <template v-else>
        <p class="gate-title">空间分布需要最新的体检数据</p>
        <p class="gate-sub">
          这份「树状分布」体量大,没有随上次结果一起缓存。重新体检一下(约 1 分钟)就能看到。
        </p>
        <button class="gate-btn" @click="rescan">重新体检</button>
      </template>
    </section>

    <template v-else-if="summary">
      <p class="tip">
        每个目录悬停右侧可「转移」到其他盘 —— 系统区和程序本体会被自动拦下,搬走的都能撤回。
      </p>
      <p v-if="summary.deniedEntries > 0" class="notice">
        有部分系统区域因权限未扫描,统计可能略有偏低。
      </p>

      <section class="tree">
        <p v-if="loading" class="empty">正在整理结果…</p>
        <p v-else-if="roots.length === 0" class="empty">没有扫描到内容。</p>
        <TreeRow
          v-else
          v-for="node in roots"
          :key="node.id"
          :node="node"
          :sibling-max="rootMax"
          :depth="0"
        />
      </section>
    </template>

    <!-- 自定义转移向导 -->
    <div
      class="mask"
      v-if="pickState !== 'closed'"
      @click.self="pickState !== 'moving' && pickState !== 'evaluating' && closePick()"
    >
      <div class="dialog">
        <!-- 评估中 -->
        <template v-if="pickState === 'evaluating'">
          <div class="gate-spin sm"></div>
          <p class="dlg-title">正在评估…</p>
          <p class="dlg-aux">检查这个目录搬走安不安全</p>
        </template>

        <!-- 确认(可转 / 不建议)-->
        <template v-else-if="pickState === 'confirm' && evalResult">
          <template v-if="evalResult.ok">
            <p class="dlg-title">转移「{{ evalResult.name }}」</p>
            <p class="warn">⚠️ 此操作有风险,请谨慎操作</p>
            <div class="ev-meta">
              <span class="bg" :class="evalResult.status">
                {{ evalResult.status === "safe" ? "安全" : "谨慎" }}
              </span>
              <span class="ev-use">{{ evalResult.displayName }}</span>
            </div>
            <p class="dlg-body sm" v-if="evalResult.note">{{ evalResult.note }}</p>
            <p class="ev-path num">{{ evalResult.path }} · {{ fmtBytes(evalResult.sizeBytes) }}</p>

            <div class="tgt-row" v-if="targets.length > 0">
              <span class="tgt-label">搬到:</span>
              <button
                v-for="t in targets"
                :key="t.mountPoint"
                class="tgt"
                :class="{ chosen: t.mountPoint === chosenTarget, bad: !t.isNtfs }"
                :disabled="!t.isNtfs"
                :title="t.isNtfs ? '' : '这个盘不是 NTFS,搬过去软件会出问题'"
                @click="chosenTarget = t.mountPoint"
              >
                {{ t.mountPoint }}（剩 {{ fmtBytes(t.freeBytes) }}）
                <em v-if="t.recommended">推荐</em>
              </button>
            </div>
            <p class="dlg-aux warn-line" v-else>
              没有可用目标盘 —— 需要一块除 C 外的 NTFS 本地硬盘(U 盘/移动硬盘不行)。
            </p>

            <p v-if="pickNotice" class="dlg-aux err">{{ pickNotice }}</p>
            <button
              class="btn wide"
              :class="{ ghosted: evalResult.status === 'cautious' }"
              :disabled="noTarget"
              @click="doMigrate"
            >
              确认转移到 {{ chosenTarget || "—" }}
            </button>
            <div class="dlg-links">
              <button class="link" @click="openFolder(evalResult.path)">打开位置看看</button>
              <button class="link" @click="closePick">先不搬了</button>
            </div>
          </template>

          <!-- 不建议 / 被硬拦 -->
          <template v-else>
            <p class="dlg-title block">这个目录不建议转移 🚫</p>
            <p class="dlg-body">{{ evalResult.note }}</p>
            <p class="ev-path num">{{ evalResult.path }} · {{ fmtBytes(evalResult.sizeBytes) }}</p>
            <p class="dlg-aux" v-if="evalResult.reason">原因:{{ evalResult.reason }}</p>
            <button class="btn wide ghost" @click="closePick">我知道了</button>
          </template>
        </template>

        <!-- 评估失败无结果 -->
        <template v-else-if="pickState === 'confirm'">
          <p class="dlg-title block">没能评估这个目录</p>
          <p class="dlg-body">{{ pickNotice || "请重新体检后再试" }}</p>
          <button class="btn wide ghost" @click="closePick">我知道了</button>
        </template>

        <!-- 搬家中 -->
        <template v-else-if="pickState === 'moving'">
          <p class="dlg-title">正在搬家…</p>
          <div class="bar"><div class="bar-fill" :style="{ width: percent + '%' }"></div></div>
          <p class="dlg-body num" v-if="progress">
            正在搬 {{ fmtBytes(progress.totalBytes) }},已完成
            {{ fmtBytes(progress.copiedBytes) }}
          </p>
          <p class="dlg-body" v-else>正在准备…</p>
          <p class="dlg-aux">别关机哦(已自动阻止休眠)。源数据全程原样保留。</p>
        </template>

        <!-- 完成 -->
        <template v-else-if="pickState === 'done' && evalResult">
          <p class="dlg-title ok">搬家完成!</p>
          <p class="dlg-body">
            「{{ evalResult.name }}」已搬到 {{ chosenTarget }},原位置留了传送门。打开相关软件确认正常后,删除
            C 盘备份才真正腾出 {{ fmtBytes(doneBytes) }}。
          </p>
          <p v-if="pickNotice" class="dlg-aux err">{{ pickNotice }}</p>
          <button class="btn wide" :disabled="confirming" @click="confirmOk">
            {{ confirming ? "正在删除备份…" : "打开正常,删备份腾空间" }}
          </button>
          <button class="link" @click="closePick">稍后再说(备份先留着)</button>
        </template>
      </div>
    </div>
  </main>
</template>

<style scoped>
.map {
  height: 100%;
  display: flex;
  flex-direction: column;
}

.head {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 4px 0 14px;
}

.back {
  color: var(--color-primary);
  font-size: var(--font-size-card-title);
}

.head-stat {
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

.tip {
  padding: 8px 12px;
  margin-bottom: 8px;
  border-radius: 8px;
  background: #e2f6ec;
  color: var(--color-action-deep);
  font-size: var(--font-size-aux);
}

.notice {
  padding: 8px 12px;
  margin-bottom: 8px;
  border-radius: 8px;
  background: var(--pill-cost-bg);
  color: var(--pill-cost-fg);
  font-size: var(--font-size-aux);
}

.tree {
  flex: 1;
  overflow-y: auto;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
  padding: 8px;
}

.empty {
  padding: 40px;
  text-align: center;
  color: var(--color-text-secondary);
}

/* 缓存态重新体检闸 */
.gate {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
  padding: 40px;
  text-align: center;
}

.gate-title {
  font-size: 17px;
  font-weight: 800;
  color: var(--color-text);
}

.gate-sub {
  font-size: 13.5px;
  color: var(--color-text-secondary);
  max-width: 420px;
  line-height: 1.7;
}

.gate-btn {
  margin-top: 8px;
  height: 46px;
  padding: 0 28px;
  border-radius: 12px;
  background: var(--color-action);
  color: #fff;
  font-size: 15px;
  font-weight: 800;
}

.gate-btn:hover {
  background: var(--color-action-deep);
}

.gate-spin {
  width: 56px;
  height: 56px;
  border-radius: 50%;
  border: 7px solid #e8eefb;
  border-top-color: var(--color-primary);
  animation: spin 0.9s linear infinite;
}

.gate-spin.sm {
  width: 40px;
  height: 40px;
  border-width: 5px;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* ── 自定义转移向导浮层 ── */
.mask {
  position: fixed;
  inset: 0;
  background: rgba(20, 28, 43, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 20;
}

.dialog {
  width: 460px;
  max-width: 92vw;
  padding: 26px 30px;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  text-align: center;
}

.dlg-title {
  font-size: var(--font-size-title);
  font-weight: 800;
}

.dlg-title.ok {
  color: var(--color-action);
}

.dlg-title.block {
  color: #e5484d;
}

.warn {
  color: var(--pill-caution-fg);
  font-weight: 800;
  font-size: 15px;
}

.dlg-body {
  color: var(--color-text);
  line-height: 1.6;
}

.dlg-body.sm {
  font-size: 13px;
  color: var(--color-text-secondary);
}

.dlg-aux {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}

.dlg-aux.err {
  color: #e5484d;
}

.dlg-aux.warn-line {
  color: var(--pill-caution-fg);
}

.ev-meta {
  display: flex;
  align-items: center;
  gap: 10px;
}

.ev-use {
  font-size: 13.5px;
  color: var(--color-text-secondary);
}

.bg {
  display: inline-flex;
  padding: 3px 11px;
  border-radius: 999px;
  font-size: 12.5px;
  font-weight: 700;
}

.bg.safe {
  background: var(--pill-safe-bg);
  color: var(--pill-safe-fg);
}

.bg.cautious {
  background: var(--pill-caution-bg);
  color: var(--pill-caution-fg);
}

.ev-path {
  font-size: 12px;
  color: var(--color-text-secondary);
  word-break: break-all;
}

.tgt-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  justify-content: center;
}

.tgt-label {
  color: var(--color-text-secondary);
  font-size: 13px;
}

.tgt {
  padding: 7px 12px;
  border-radius: 10px;
  border: 2px solid var(--color-line);
  background: var(--color-card);
  font-size: 13px;
}

.tgt.chosen {
  border-color: var(--color-primary);
  color: var(--color-primary);
  font-weight: 700;
}

.tgt.bad {
  opacity: 0.55;
  cursor: not-allowed;
}

.tgt em {
  font-style: normal;
  font-size: var(--font-size-aux);
  color: var(--color-action);
  margin-left: 4px;
}

.dlg-links {
  display: flex;
  gap: 18px;
}

.btn {
  height: 46px;
  padding: 0 24px;
  border-radius: 10px;
  background: var(--color-action);
  color: #fff;
  font-size: 15px;
  font-weight: 800;
  transition: background 0.16s;
}

.btn:hover:not(:disabled) {
  background: var(--color-action-deep);
}

.btn:disabled {
  background: #d1d5db;
  cursor: not-allowed;
}

.btn.wide {
  min-width: 260px;
}

.btn.ghosted {
  background: var(--color-card);
  color: var(--pill-caution-fg);
  border: 1.5px solid var(--pill-caution-fg);
}

.btn.ghosted:hover:not(:disabled) {
  background: #fff4ea;
}

.btn.ghost {
  background: var(--color-card);
  color: var(--color-text);
  border: 1.5px solid var(--color-line);
}

.link {
  color: var(--color-primary);
  font-size: var(--font-size-body);
}

.bar {
  width: 100%;
  height: 10px;
  border-radius: 5px;
  background: #e8eefb;
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  border-radius: 5px;
  background: var(--color-action);
  transition: width 0.2s;
}
</style>
