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
  video: "🎬",
  archive: "🗜️",
  installer: "📦",
  image: "💿",
  other: "📄",
};

const shown = computed(() =>
  filter.value === "all" ? files.value : files.value.filter((f) => f.category === filter.value),
);
const totalBytes = computed(() => files.value.reduce((s, f) => s + f.sizeBytes, 0));

onMounted(async () => {
  if (!scanSummary.value) {
    router.replace("/");
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
  <main class="big">
    <header class="head">
      <button class="back" @click="router.push('/report')">‹ 返回报告</button>
      <div class="head-stat">
        <span class="head-title">大文件排查</span>
        <span class="head-sub num" v-if="!loading">
          {{ files.length }} 个 100MB 以上的文件 · 共 {{ fmtBytes(totalBytes) }} · 删不删由你决定
        </span>
      </div>
    </header>

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

    <p v-if="loading" class="empty">正在整理…</p>
    <p v-else-if="shown.length === 0" class="empty">
      没有找到 100MB 以上的大文件,你的 C 盘很干净。
    </p>

    <section class="list" v-else>
      <div v-for="f in shown" :key="f.path" class="row" :title="f.path">
        <span class="icon">{{ CAT_ICON[f.category] }}</span>
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
    </section>
  </main>
</template>

<style scoped>
.big {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 0 24px 24px;
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

.recycle-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  margin-bottom: 8px;
  border-radius: 8px;
  background: #fef3c7;
  color: var(--color-warning);
}

.mini-btn {
  padding: 4px 12px;
  border-radius: 6px;
  background: var(--color-warning);
  color: #fff;
  font-weight: 600;
}

.notice {
  padding: 8px 14px;
  margin-bottom: 8px;
  border-radius: 8px;
  background: #dcfce7;
  color: var(--color-success);
}

.filters {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
}

.chip {
  padding: 6px 16px;
  border-radius: 999px;
  border: 1px solid #e5e7eb;
  background: var(--color-card);
  color: var(--color-text-secondary);
}

.chip.on {
  border-color: var(--color-primary);
  color: var(--color-primary);
  font-weight: 600;
}

.list {
  flex: 1;
  overflow-y: auto;
  background: var(--color-card);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-card);
  padding: 4px 0;
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  border-bottom: 1px solid #f3f4f6;
}

.row:last-child {
  border-bottom: none;
}

.icon {
  font-size: 20px;
  flex-shrink: 0;
}

.info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.name {
  font-weight: 600;
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
  font-weight: 700;
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
  border-radius: 6px;
  border: 1px solid #e5e7eb;
  background: var(--color-card);
}

.op.del {
  color: var(--color-warning);
  border-color: var(--color-warning);
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
  color: var(--color-warning);
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
