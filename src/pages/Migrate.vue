<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  checkLocks,
  confirmMigration,
  getMigratables,
  getMigrateCandidates,
  getMigrateTargets,
  onMigrateProgress,
  requestClose,
  startMigrate,
  cancelMigrate,
} from "../api";
import type {
  KnownFolderInfo,
  MigratableItem,
  MigrateCandidate,
  MigrateProgress,
  TargetDisk,
} from "../api/types";
import { scanSummary } from "../store";
import { fmtBytes } from "../utils/format";

const router = useRouter();

const items = ref<MigratableItem[]>([]);
const candidates = ref<MigrateCandidate[]>([]);
const knownFolders = ref<KnownFolderInfo[]>([]);
const targets = ref<TargetDisk[]>([]);
const chosenTarget = ref<string>("");
const loading = ref(true);
const notice = ref("");

const hasScan = computed(() => !!scanSummary.value);

/* 搬家目标(ruleId 对 KB 项、pick:<hash> 对自选项,后端统一解析) */
interface Target {
  ruleId: string;
  displayName: string;
}

/* 向导状态机:idle=列表 / waiting=请退出软件 / moving=搬家中 / done=完成 */
const wizard = ref<"idle" | "waiting" | "moving" | "done">("idle");
const active = ref<Target | null>(null);
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
    loading.value = false;
    return;
  }
  try {
    const [mig, cand, tgs] = await Promise.all([
      getMigratables(),
      getMigrateCandidates().catch(() => ({ candidates: [], knownFolders: [] })),
      getMigrateTargets(),
    ]);
    items.value = mig;
    candidates.value = cand.candidates;
    knownFolders.value = cand.knownFolders;
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
async function begin(ruleId: string, displayName: string) {
  if (!chosenTarget.value) {
    notice.value = "没有找到能用的目标盘(需要一块 NTFS 格式的本地硬盘)";
    return;
  }
  notice.value = "";
  active.value = { ruleId, displayName };
  const st = await checkLocks([ruleId]).catch(() => []);
  lockedBy.value = st.find((s) => s.ruleId === ruleId)?.lockedBy ?? [];
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
  const finishedId = active.value.ruleId;
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
    // 搬走的自选项从候选列表移除(它现在是联接了)
    candidates.value = candidates.value.filter((c) => c.id !== finishedId);
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

async function openFolder(path: string) {
  try {
    await revealItemInDir(path);
  } catch {
    notice.value = "没能打开这个文件夹";
  }
}

const noTarget = computed(() => targets.value.length === 0 || !chosenTarget.value);
const nothing = computed(
  () =>
    items.value.length === 0 &&
    candidates.value.length === 0 &&
    knownFolders.value.length === 0,
);
</script>

<template>
  <div class="page">
    <!-- 未体检:引导回概览(侧栏时代页面常驻,不强制跳转) -->
    <section class="rcard guide-card" v-if="!hasScan && !loading">
      <span class="tile sm"><svg class="ic"><use href="#i-move" /></svg></span>
      <div class="tx">
        <b>先做一次体检</b>
        <p>体检后这里会列出能搬到其他盘的大目录,原位置留「传送门」,软件照常使用</p>
      </div>
      <button class="btn" @click="router.push('/')">去概览体检 ›</button>
    </section>

    <template v-else>
      <div class="phead">
        <div class="phead-l">
          <div class="ptitle">搬家瘦身</div>
          <div class="psub">把大目录搬到其他盘,原位置留一个「传送门」,软件照常使用</div>
        </div>
        <button class="link" @click="router.push('/moved')">已搬家管理 ›</button>
      </div>

      <p v-if="notice" class="notice">{{ notice }}</p>

      <p v-if="loading" class="empty">正在整理…</p>
      <p v-else-if="nothing" class="empty">
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
            {{ t.mountPoint }}（剩 {{ fmtBytes(t.freeBytes) }}）
            <em v-if="t.recommended">推荐</em>
            <em v-if="!t.isNtfs" class="bad-tag">不支持</em>
          </button>
        </section>
        <p v-else class="notice">
          没有找到能用的目标盘——需要一块除 C 盘外的本地硬盘(U 盘和移动硬盘不行,拔掉后软件会打不开)。
        </p>

        <!-- 区一:推荐搬家(知识库命中,最稳) -->
        <section class="rcard" v-if="items.length > 0">
          <div class="sec-head">
            <span class="sec-title">推荐搬家</span>
            <span class="sec-sub">已识别的软件数据目录,搬家最稳妥</span>
          </div>
          <div class="rows">
            <div v-for="item in items" :key="item.ruleId" class="row">
              <div class="ib">
                <div class="l1">
                  <span class="nm">{{ item.displayName }}</span>
                  <span class="sz num">{{ fmtBytes(item.sizeBytes) }}</span>
                </div>
                <p class="ex">{{ item.explain }}</p>
              </div>
              <button
                class="btn item-btn"
                :disabled="noTarget"
                @click="begin(item.ruleId, item.displayName)"
              >
                开始搬家
              </button>
            </div>
          </div>
        </section>

        <!-- 区二:大文件夹自选(知识库外,用户手动决定) -->
        <section class="rcard" v-if="candidates.length > 0">
          <div class="sec-head">
            <span class="sec-title">大文件夹自选</span>
            <span class="sec-sub">
              体检发现的其他大文件夹,默认都不动;搬走的数据全程保留、可一键搬回。不认识的先别动。
            </span>
          </div>
          <div class="rows">
            <div
              v-for="c in candidates"
              :key="c.id"
              class="row"
              :class="{ off: c.status === 'blocked' }"
              :title="c.path"
            >
              <div class="ib">
                <div class="l1">
                  <span class="nm">{{ c.name }}</span>
                  <span class="sz num">{{ fmtBytes(c.sizeBytes) }}</span>
                </div>
                <p class="ex">{{ c.displayName }}</p>
                <div class="tags">
                  <span v-if="c.status === 'cautious'" class="bg caution">谨慎</span>
                  <span v-if="c.note" class="note">{{ c.note }}</span>
                  <button class="reveal" @click="openFolder(c.path)">打开位置看看 ›</button>
                </div>
              </div>
              <button
                v-if="c.status !== 'blocked'"
                class="btn item-btn"
                :class="{ ghosted: c.status === 'cautious' }"
                :disabled="noTarget"
                @click="begin(c.id, c.name)"
              >
                搬走它
              </button>
              <span v-else class="blocked-tag">不支持</span>
            </div>
          </div>
        </section>

        <!-- 区三:官方搬法(Known Folder 有官方位置重定向,junction 是错误工具) -->
        <section class="rcard guide-sec" v-if="knownFolders.length > 0">
          <div class="sec-head">
            <span class="sec-title">这些用官方方法搬更好</span>
            <span class="sec-sub">
              下载、视频这类文件夹,Windows 自带更换位置的功能,比本工具更适合搬它们
            </span>
          </div>
          <div class="rows">
            <div v-for="k in knownFolders" :key="k.path" class="row">
              <div class="ib">
                <div class="l1">
                  <span class="nm">「{{ k.name }}」文件夹</span>
                  <span class="sz num">{{ fmtBytes(k.sizeBytes) }}</span>
                </div>
                <p class="ex">
                  右键这个文件夹 →「属性」→「位置」→「移动」,选到其他盘即可,系统会自动搬好。
                </p>
                <button class="reveal" @click="openFolder(k.path)">打开这个文件夹 ›</button>
              </div>
            </div>
          </div>
        </section>
      </template>
    </template>

    <!-- 向导浮层 -->
    <div class="mask" v-if="wizard !== 'idle' && active">
      <div class="dialog">
        <!-- 步骤1:请退出软件 -->
        <template v-if="wizard === 'waiting'">
          <p class="dlg-title">先退出{{ active.displayName.slice(0, 10) }}相关软件</p>
          <template v-if="lockedBy.length > 0">
            <p class="dlg-body">
              {{ lockedBy.join("、") }} 正在使用这些文件。请手动退出,或让我来:
            </p>
            <button class="btn wide" :disabled="closing" @click="helpClose">
              {{ closing ? "正在请它退出…" : "帮我退出" }}
            </button>
            <p class="dlg-aux">退出后这里会自动亮起,不用刷新</p>
          </template>
          <template v-else>
            <p class="dlg-body ok">✓ 没有软件占用,可以开始搬家了</p>
            <button class="btn wide" @click="doMigrate">开始搬家</button>
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
              active.displayName.slice(0, 10)
            }}试试——一切正常的话,点下面按钮删除 C 盘备份,才会真正腾出
            {{ fmtBytes(doneBytes) }}。
          </p>
          <button class="btn wide" :disabled="confirming" @click="confirmOk">
            {{ confirming ? "正在删除备份…" : "打开正常,删除备份腾空间" }}
          </button>
          <button class="link" @click="later">稍后再说(备份先留着)</button>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.phead {
  display: flex;
  align-items: flex-end;
  gap: 16px;
  padding: 2px 4px 0;
}

.phead-l {
  flex: 1;
  min-width: 0;
}

.ptitle {
  font-size: 21px;
  font-weight: 900;
  color: var(--color-text);
}

.psub {
  font-size: 13.5px;
  color: var(--color-text-secondary);
  margin-top: 2px;
}

.link {
  color: var(--color-primary);
  font-size: var(--font-size-body);
  flex-shrink: 0;
}

.rcard {
  background: var(--color-card);
  border-radius: var(--radius-card);
  padding: 18px 26px;
  box-shadow: var(--shadow-card);
}

.guide-card {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 24px 26px;
}

.guide-sec {
  background: #f3f7ff;
  border: 1px solid #dbe7fb;
  box-shadow: none;
}

.sec-head {
  margin-bottom: 8px;
}

.sec-title {
  font-size: 15px;
  font-weight: 800;
  color: var(--color-text);
}

.sec-sub {
  display: block;
  font-size: 12.5px;
  color: var(--color-text-secondary);
  margin-top: 2px;
  line-height: 1.6;
}

.tile {
  border-radius: 13px;
  background: #e2f6ec;
  color: var(--color-action);
  display: grid;
  place-items: center;
  flex-shrink: 0;
}

.tile.sm {
  width: 46px;
  height: 46px;
  font-size: 22px;
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

.tx p {
  font-size: 13.5px;
  color: var(--color-text-secondary);
  margin-top: 2px;
}

.notice {
  padding: 10px 14px;
  border-radius: 10px;
  background: var(--pill-safe-bg);
  color: var(--pill-safe-fg);
}

.targets {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.targets-label {
  color: var(--color-text-secondary);
}

.target {
  padding: 8px 14px;
  border-radius: 10px;
  border: 2px solid var(--color-line);
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
  color: var(--color-action);
  margin-left: 4px;
}

.target em.bad-tag {
  color: var(--color-text-secondary);
}

.rows {
  display: flex;
  flex-direction: column;
}

.row {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 15px 0;
  border-top: 1px solid var(--color-line);
}

.rows .row:first-child {
  border-top: none;
}

.row.off {
  opacity: 0.62;
}

.ib {
  flex: 1;
  min-width: 0;
}

.l1 {
  display: flex;
  justify-content: space-between;
  gap: 12px;
}

.nm {
  font-size: 15.5px;
  font-weight: 700;
}

.sz {
  font-size: 16px;
  font-weight: 900;
}

.ex {
  font-size: 13.5px;
  color: var(--color-text-secondary);
  margin: 2px 0;
}

.tags {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 4px;
}

.bg {
  display: inline-flex;
  align-items: center;
  padding: 3px 11px;
  border-radius: 999px;
  font-size: 12.5px;
  font-weight: 600;
}

.bg.caution {
  background: var(--pill-caution-bg);
  color: var(--pill-caution-fg);
}

.note {
  font-size: 12.5px;
  color: var(--color-text-secondary);
}

.reveal {
  font-size: 12.5px;
  padding: 0;
  color: var(--color-primary);
}

.blocked-tag {
  flex-shrink: 0;
  font-size: 13px;
  color: var(--color-text-secondary);
  padding: 0 8px;
}

/* 主行动按钮:行动绿(设计规范 §4.2:主按钮由蓝改绿) */
.btn {
  height: 44px;
  padding: 0 22px;
  border-radius: 10px;
  background: var(--color-action);
  color: #fff;
  font-size: 14.5px;
  font-weight: 700;
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

/* 谨慎项按钮降一档视觉,不用满绿诱导 */
.btn.ghosted {
  background: var(--color-card);
  color: var(--pill-caution-fg);
  border: 1px solid var(--pill-caution-fg);
}

.btn.ghosted:hover:not(:disabled) {
  background: #fff4ea;
}

.btn.wide {
  height: 48px;
  min-width: 240px;
  font-size: 15.5px;
  font-weight: 800;
}

.item-btn {
  flex-shrink: 0;
}

.mask {
  position: fixed;
  inset: 0;
  background: rgba(20, 28, 43, 0.4);
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
  box-shadow: var(--shadow-card);
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
  color: var(--color-action);
}

.dlg-body {
  color: var(--color-text);
}

.dlg-body.ok {
  color: var(--color-action);
  font-weight: 600;
}

.dlg-aux {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
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

.empty {
  padding: 40px;
  text-align: center;
  color: var(--color-text-secondary);
}
</style>
