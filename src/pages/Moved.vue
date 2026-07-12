<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  confirmMigration,
  getExternalJunctions,
  getMigrations,
  onMigrateProgress,
  revertExternalJunction,
  revertMigration,
} from "../api";
import type { ExternalJunction, MigrateProgress, MigrateRecord } from "../api/types";
import { fmtBytes } from "../utils/format";

const router = useRouter();

const records = ref<MigrateRecord[]>([]);
/* 其他已搬走的目录(非本工具搬的,指向别的盘的 junction) */
const externals = ref<ExternalJunction[]>([]);
const loading = ref(true);
const notice = ref("");
const noticeKind = ref<"ok" | "warn">("ok");
/* 行内二次确认:待确认搬回的 ruleId(不可逆感知,禁用「确定/取消」措辞) */
const revertAsk = ref<string | null>(null);
/* 外部 junction 的行内确认与搬回中标记(用 src 作 key) */
const revertExtAsk = ref<string | null>(null);
const revertingExt = ref<string | null>(null);
const busy = ref<string | null>(null);
const reverting = ref<string | null>(null);
const progress = ref<MigrateProgress | null>(null);

let unlisten: UnlistenFn | null = null;

const percent = computed(() => {
  if (!progress.value || progress.value.totalBytes === 0) return 0;
  return Math.min(99, (progress.value.copiedBytes / progress.value.totalBytes) * 100);
});

async function refresh() {
  const [recs, exts] = await Promise.all([
    getMigrations().catch(() => [] as MigrateRecord[]),
    getExternalJunctions().catch(() => [] as ExternalJunction[]),
  ]);
  records.value = recs;
  externals.value = exts;
}

onMounted(async () => {
  await refresh();
  loading.value = false;
});

onUnmounted(() => {
  unlisten?.();
});

function say(msg: string, kind: "ok" | "warn" = "ok") {
  notice.value = msg;
  noticeKind.value = kind;
}

async function deleteBak(rec: MigrateRecord) {
  if (busy.value) return;
  busy.value = rec.ruleId;
  try {
    await confirmMigration(rec.ruleId);
    say(`已腾出 ${fmtBytes(rec.bytes)}`);
    await refresh();
  } catch (e) {
    say(String(e), "warn");
  } finally {
    busy.value = null;
  }
}

async function doRevert(rec: MigrateRecord) {
  if (busy.value) return;
  revertAsk.value = null;
  busy.value = rec.ruleId;
  reverting.value = rec.ruleId;
  progress.value = null;
  try {
    unlisten = await onMigrateProgress((p) => {
      progress.value = p;
    });
    await revertMigration(rec.ruleId);
    say(`已搬回 C 盘,${rec.displayName}回到搬家前的样子`);
    await refresh();
  } catch (e) {
    say(String(e), "warn");
  } finally {
    unlisten?.();
    unlisten = null;
    busy.value = null;
    reverting.value = null;
  }
}

/* 搬回外部 junction(非本工具搬的):复用同一事务与进度机制 */
async function doRevertExt(ext: ExternalJunction) {
  if (busy.value) return;
  revertExtAsk.value = null;
  busy.value = ext.src;
  revertingExt.value = ext.src;
  progress.value = null;
  try {
    unlisten = await onMigrateProgress((p) => {
      progress.value = p;
    });
    await revertExternalJunction(ext.src);
    say(`已搬回 C 盘,${ext.name} 回到 C 盘了`);
    await refresh();
  } catch (e) {
    say(String(e), "warn");
  } finally {
    unlisten?.();
    unlisten = null;
    busy.value = null;
    revertingExt.value = null;
  }
}
</script>

<template>
  <main class="moved">
    <header class="head">
      <button class="back" @click="router.push('/migrate')">‹ 返回</button>
      <div class="head-stat">
        <span class="head-title">已搬家管理</span>
        <span class="head-sub">软件更新后如有异常,在这里一键搬回就能恢复</span>
      </div>
    </header>

    <p v-if="notice" class="notice" :class="noticeKind">{{ notice }}</p>

    <p v-if="loading" class="empty">正在读取…</p>
    <p v-else-if="records.length === 0 && externals.length === 0" class="empty">
      还没有搬过家的目录。到「搬家瘦身」页把大目录搬到其他盘,给 C 盘腾地方。
    </p>

    <template v-else>
      <section class="list" v-if="records.length > 0">
        <div v-for="rec in records" :key="rec.ruleId + rec.src" class="item">
          <div class="item-body">
            <div class="item-line1">
              <span class="item-name">{{ rec.displayName }}</span>
              <span class="item-size num">{{ fmtBytes(rec.bytes) }}</span>
            </div>
            <p class="item-path" :title="rec.src + ' → ' + rec.dst">
              {{ rec.src }} → {{ rec.dst }}
            </p>
            <p class="item-aux">
              搬家时间 {{ rec.at }}
              <template v-if="rec.bak">
                · C 盘备份还在(确认软件正常后删除,才腾出空间)
              </template>
            </p>

            <!-- 搬回进度 -->
            <template v-if="reverting === rec.ruleId">
              <div class="bar">
                <div class="bar-fill" :style="{ width: percent + '%' }"></div>
              </div>
              <p class="item-aux num" v-if="progress">
                正在搬回 {{ fmtBytes(progress.copiedBytes) }} /
                {{ fmtBytes(progress.totalBytes) }},别关机哦
              </p>
            </template>
          </div>

          <div class="item-ops" v-if="reverting !== rec.ruleId">
            <button
              v-if="rec.bak"
              class="op primary"
              :disabled="busy !== null"
              @click="deleteBak(rec)"
            >
              {{ busy === rec.ruleId ? "正在删除…" : `删除备份,腾出 ${fmtBytes(rec.bytes)}` }}
            </button>

            <template v-if="revertAsk === rec.ruleId">
              <span class="ask">把数据复制回 C 盘?需要 C 盘有 {{ fmtBytes(rec.bytes) }} 空间</span>
              <button class="op danger" :disabled="busy !== null" @click="doRevert(rec)">
                搬回
              </button>
              <button class="op" @click="revertAsk = null">先不搬</button>
            </template>
            <button
              v-else
              class="op"
              :disabled="busy !== null"
              @click="revertAsk = rec.ruleId"
            >
              搬回 C 盘
            </button>
          </div>
        </div>
      </section>

      <!-- 其他已搬走的目录:不是本工具搬的,但指向别的盘、可以搬回 -->
      <section class="ext-block" v-if="externals.length > 0">
        <div class="ext-head">
          <span class="ext-title">其他已搬走的目录</span>
          <span class="ext-sub">
            这些目录之前被(用别的方式)搬到了其他盘,本工具也能帮你搬回 C 盘
          </span>
        </div>
        <div class="list">
          <div v-for="ext in externals" :key="ext.src" class="item">
            <div class="item-body">
              <div class="item-line1">
                <span class="item-name">{{ ext.name }}</span>
                <span class="item-size num">{{ fmtBytes(ext.sizeBytes) }}</span>
              </div>
              <p class="item-path" :title="ext.src + ' → ' + ext.dst">
                {{ ext.src }} → {{ ext.dst }}
              </p>

              <template v-if="revertingExt === ext.src">
                <div class="bar">
                  <div class="bar-fill" :style="{ width: percent + '%' }"></div>
                </div>
                <p class="item-aux num" v-if="progress">
                  正在搬回 {{ fmtBytes(progress.copiedBytes) }} /
                  {{ fmtBytes(progress.totalBytes) }},别关机哦
                </p>
              </template>
            </div>

            <div class="item-ops" v-if="revertingExt !== ext.src">
              <template v-if="revertExtAsk === ext.src">
                <span class="ask">
                  把数据搬回 C 盘?需要 C 盘有 {{ fmtBytes(ext.sizeBytes) }} 空间
                </span>
                <button class="op danger" :disabled="busy !== null" @click="doRevertExt(ext)">
                  搬回
                </button>
                <button class="op" @click="revertExtAsk = null">先不搬</button>
              </template>
              <button
                v-else
                class="op"
                :disabled="busy !== null"
                @click="revertExtAsk = ext.src"
              >
                搬回 C 盘
              </button>
            </div>
          </div>
        </div>
      </section>
    </template>
  </main>
</template>

<style scoped>
.moved {
  display: flex;
  flex-direction: column;
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

.notice {
  padding: 10px 14px;
  margin-bottom: 12px;
  border-radius: 8px;
}

.notice.ok {
  background: var(--pill-safe-bg);
  color: var(--pill-safe-fg);
}

.notice.warn {
  background: var(--pill-cost-bg);
  color: var(--pill-cost-fg);
}

/* 其他已搬走的目录区块 */
.ext-block {
  margin-top: 20px;
}

.ext-head {
  padding: 0 2px 10px;
}

.ext-title {
  font-size: 15px;
  font-weight: 800;
  color: var(--color-text);
}

.ext-sub {
  display: block;
  font-size: 12.5px;
  color: var(--color-text-secondary);
  margin-top: 2px;
  line-height: 1.6;
}

.list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.item {
  display: flex;
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

.item-path {
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  margin: 2px 0;
}

.item-aux {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}

.item-ops {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  justify-content: center;
  gap: 8px;
}

.op {
  padding: 8px 16px;
  border-radius: 8px;
  border: 1px solid var(--color-line);
  background: var(--color-card);
  color: var(--color-text);
}

.op.primary {
  background: var(--color-action);
  border-color: var(--color-action);
  color: #fff;
  font-weight: 600;
}

.op.danger {
  background: var(--pill-cost-fg);
  border-color: var(--pill-cost-fg);
  color: #fff;
  font-weight: 600;
}

.op:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.ask {
  font-size: var(--font-size-aux);
  color: var(--color-warning);
  max-width: 220px;
  text-align: right;
}

.bar {
  width: 100%;
  height: 8px;
  border-radius: 4px;
  background: #e8eefb;
  overflow: hidden;
  margin-top: 8px;
}

.bar-fill {
  height: 100%;
  border-radius: 4px;
  background: var(--color-action);
  transition: width 0.2s;
}

.empty {
  padding: 40px;
  text-align: center;
  color: var(--color-text-secondary);
  line-height: 1.8;
}
</style>
