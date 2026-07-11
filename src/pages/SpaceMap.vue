<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import TreeRow from "../components/TreeRow.vue";
import { getChildren } from "../api";
import type { TreeNode } from "../api/types";
import { scanSummary } from "../store";
import { fmtBytes } from "../utils/format";

const router = useRouter();
const roots = ref<TreeNode[]>([]);
const loading = ref(true);

const summary = computed(() => scanSummary.value);
const rootMax = computed(() => roots.value.reduce((m, c) => Math.max(m, c.sizeBytes), 0));

onMounted(async () => {
  // 直接进入或刷新丢失扫描态时,退回首页重新体检
  if (!summary.value) {
    router.replace("/");
    return;
  }
  try {
    roots.value = await getChildren(summary.value.rootId);
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <main class="map" v-if="summary">
    <header class="head">
      <button class="back" @click="router.push('/report')">‹ 返回报告</button>
      <div class="head-stat">
        <span class="head-title">C盘空间分布</span>
        <span class="head-sub num">
          共 {{ fmtBytes(summary.totalBytes) }} · 扫描用时 {{ (summary.elapsedMs / 1000).toFixed(1) }} 秒
        </span>
      </div>
    </header>

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
  </main>
</template>

<style scoped>
.map {
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
  position: sticky;
  top: 0;
  background: var(--color-bg);
  z-index: 1;
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
  background: #fef3c7;
  color: var(--color-warning);
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
</style>
