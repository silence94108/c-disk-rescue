<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import TreeRow from "../components/TreeRow.vue";
import { getChildren } from "../api";
import type { TreeNode } from "../api/types";
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

onMounted(() => {
  // 缓存态(有上次结果但无 live 树)不直接请求,展示重新体检闸
  if (summary.value && !isStale.value) {
    load();
  } else {
    loading.value = false;
  }
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

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
