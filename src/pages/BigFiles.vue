<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { deleteBigFile, getBigFiles, runClean } from "../api";
import type { BigFileInfo, FileCategory } from "../api/types";
import { scanSummary } from "../store";
import { fmtBytes, fmtDate } from "../utils/format";

const router = useRouter();

const files = ref<BigFileInfo[]>([]);
const loading = ref(true);
const filter = ref<"all" | FileCategory>("all");
/* 行内删除确认:待确认的路径(>2GB 走红色永久删除警示) */
const askDelete = ref<string | null>(null);
const busy = ref<string | null>(null);
/* 删除后的回收站提示条:累计移入的字节数 */
const recycled = ref(0);
const emptying = ref(false);
const notice = ref("");

const hasScan = computed(() => !!scanSummary.value);

const HUGE = 2 * 1024 ** 3;

const CATS: { key: "all" | FileCategory; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "video", label: "视频" },
  { key: "archive", label: "压缩包" },
  { key: "installer", label: "安装包" },
  { key: "image", label: "镜像" },
  { key: "other", label: "其他" },
];

const CAT_ICON: Record<FileCategory, string> = {
  video: "#i-video",
  archive: "#i-zip",
  installer: "#i-installer",
  image: "#i-disc",
  other: "#i-file",
};

const shown = computed(() =>
  filter.value === "all" ? files.value : files.value.filter((f) => f.category === filter.value),
);
const totalBytes = computed(() => files.value.reduce((s, f) => s + f.sizeBytes, 0));

onMounted(async () => {
  if (!scanSummary.value) {
    loading.value = false;
    return;
  }
  try {
    files.value = await getBigFiles();
  } finally {
    loading.value = false;
  }
});

async function reveal(f: BigFileInfo) {
  try {
    await revealItemInDir(f.path);
  } catch {
    notice.value = "没能打开文件位置";
  }
}

async function doDelete(f: BigFileInfo) {
  if (busy.value) return;
  askDelete.value = null;
  busy.value = f.path;
  try {
    await deleteBigFile(f.path);
    files.value = files.value.filter((x) => x.path !== f.path);
    recycled.value += f.sizeBytes;
  } catch (e) {
    notice.value = String(e);
  } finally {
    busy.value = null;
  }
}

/* 一键清空回收站:复用清理引擎的 recycle-bin 规则 */
async function emptyRecycleBin() {
  if (emptying.value) return;
  emptying.value = true;
  try {
    const r = await runClean(["recycle-bin"]);
    notice.value = `已清空回收站,真正腾出 ${fmtBytes(r.freedBytes)}`;
    recycled.value = 0;
  } catch (e) {
    notice.value = String(e);
  } finally {
    emptying.value = false;
  }
}
</script>

<template>
  <div class="page">
    <!-- 未体检:引导回概览(侧栏时代页面常驻,不强制跳转) -->
    <section class="rcard guide-card" v-if="!hasScan && !loading">
      <span class="tile sm"><svg class="ic"><use href="#i-file" /></svg></span>
      <div class="tx">
        <b>先做一次体检</b>
        <p>体检后这里会按大小列出 100MB 以上的文件,帮你找回被遗忘的空间</p>
      </div>
      <button class="btn" @click="router.push('/')">去概览体检 ›</button>
    </section>

    <template v-else>
      <div class="phead">
        <div class="ptitle">大文件</div>
        <div class="psub num" v-if="!loading">
          {{ files.length }} 个 100MB 以上的文件 · 共 {{ fmtBytes(totalBytes) }} · 删不删由你决定
        </div>
      </div>

      <!-- 回收站提示:进回收站 ≠ 释放空间,不讲明白用户会觉得工具无效(设计规范 §3.5) -->
      <p v-if="recycled > 0" class="recycle-bar">
        已移到回收站 {{ fmtBytes(recycled) }}(空间还没释放),清空回收站后才真正腾出来
        <button class="mini-btn" :disabled="emptying" @click="emptyRecycleBin">
          {{ emptying ? "正在清空…" : "立即清空" }}
        </button>
      </p>
      <p v-if="notice" class="notice">{{ notice }}</p>

      <div class="filters">
        <button
          v-for="c in CATS"
          :key="c.key"
          class="chip"
          :class="{ on: filter === c.key }"
          @click="filter = c.key"
        >
          {{ c.label }}
        </button>
      </div>

      <section class="rcard listcard">
        <p v-if="loading" class="empty">正在整理…</p>
        <p v-else-if="shown.length === 0" class="empty">
          没有找到 100MB 以上的大文件,你的 C 盘很干净。
        </p>
        <div class="rows" v-else>
          <div v-for="f in shown" :key="f.path" class="row" :title="f.path">
            <span class="ficon"><svg class="ic"><use :href="CAT_ICON[f.category]" /></svg></span>
            <div class="info">
              <span class="name">{{ f.name }}</span>
              <span class="path">{{ f.path }}</span>
            </div>
            <span class="mtime num">{{ fmtDate(f.modifiedMs) }}</span>
            <span class="size num">{{ fmtBytes(f.sizeBytes) }}</span>

            <div class="ops">
              <button class="op" @click="reveal(f)">打开位置</button>

              <template v-if="!f.deletable">
                <span class="no-del" :title="f.reason ?? ''">不能删 ⓘ</span>
              </template>
              <template v-else-if="askDelete === f.path">
                <!-- 超大文件可能放不进回收站转永久删除,红色二次确认(设计规范 §3.5) -->
                <span class="ask" :class="{ danger: f.sizeBytes >= HUGE }">
                  {{
                    f.sizeBytes >= HUGE
                      ? "这个文件太大可能放不进回收站,会被直接删除、无法找回,确定吗?"
                      : "删除后可在回收站找回,确定吗?"
                  }}
                </span>
                <button
                  class="op del"
                  :class="{ danger: f.sizeBytes >= HUGE }"
                  :disabled="busy !== null"
                  @click="doDelete(f)"
                >
                  {{ busy === f.path ? "删除中…" : f.sizeBytes >= HUGE ? "永久删除" : "删除" }}
                </button>
                <button class="op" @click="askDelete = null">先不删</button>
              </template>
              <button v-else class="op" :disabled="busy !== null" @click="askDelete = f.path">
                删除
              </button>
            </div>
          </div>
        </div>
      </section>
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
}

.phead {
  padding: 2px 4px 0;
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

.rcard {
  background: var(--color-card);
  border-radius: var(--radius-card);
  padding: 24px 26px;
  box-shadow: var(--shadow-card);
}

.guide-card {
  display: flex;
  align-items: center;
  gap: 16px;
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

.btn {
  height: 48px;
  padding: 0 26px;
  border-radius: 12px;
  background: var(--color-action);
  color: #fff;
  font-size: 15.5px;
  font-weight: 800;
  white-space: nowrap;
}

.btn:hover {
  background: var(--color-action-deep);
}

.recycle-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  border-radius: 10px;
  background: var(--pill-cost-bg);
  color: var(--pill-cost-fg);
}

.mini-btn {
  padding: 4px 12px;
  border-radius: 6px;
  background: var(--pill-cost-fg);
  color: #fff;
  font-weight: 600;
}

.notice {
  padding: 8px 14px;
  border-radius: 10px;
  background: var(--pill-safe-bg);
  color: var(--pill-safe-fg);
}

.filters {
  display: flex;
  gap: 8px;
}

.chip {
  padding: 6px 16px;
  border-radius: 999px;
  border: 1px solid var(--color-line);
  background: var(--color-card);
  color: var(--color-text-secondary);
}

.chip.on {
  border-color: var(--color-primary);
  color: var(--color-primary);
  font-weight: 600;
}

.listcard {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 18px;
}

.rows {
  display: flex;
  flex-direction: column;
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 4px;
  border-top: 1px solid var(--color-line);
}

.rows .row:first-child {
  border-top: none;
}

.ficon {
  font-size: 20px;
  color: #8a93a6;
  display: flex;
  flex-shrink: 0;
}

.info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.name {
  font-weight: 700;
  font-size: 14.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.path {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mtime {
  flex-shrink: 0;
  width: 88px;
  color: var(--color-text-secondary);
  font-size: var(--font-size-aux);
}

.size {
  flex-shrink: 0;
  width: 80px;
  text-align: right;
  font-weight: 900;
}

.ops {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  justify-content: flex-end;
}

.op {
  padding: 6px 12px;
  border-radius: 8px;
  border: 1px solid var(--color-line);
  background: var(--color-card);
}

.op.del {
  color: var(--pill-cost-fg);
  border-color: var(--pill-cost-fg);
}

.op.del.danger {
  background: var(--color-danger);
  border-color: var(--color-danger);
  color: #fff;
  font-weight: 600;
}

.op:disabled {
  opacity: 0.5;
}

.ask {
  font-size: var(--font-size-aux);
  color: var(--pill-cost-fg);
  max-width: 260px;
}

.ask.danger {
  color: var(--color-danger);
  font-weight: 600;
}

.no-del {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
  cursor: help;
}

.empty {
  padding: 40px;
  text-align: center;
  color: var(--color-text-secondary);
}
</style>
