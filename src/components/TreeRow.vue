<script setup lang="ts">
import { ref } from "vue";
import type { TreeNode } from "../api/types";
import { getChildren } from "../api";
import { fmtBytes, fmtCount } from "../utils/format";

const props = defineProps<{
  node: TreeNode;
  /** 同级最大 size,用于占比条归一化 */
  siblingMax: number;
  depth: number;
}>();

const expanded = ref(false);
const loading = ref(false);
const children = ref<TreeNode[]>([]);
const loaded = ref(false);

const barPercent = () =>
  props.siblingMax > 0 ? Math.max((props.node.sizeBytes / props.siblingMax) * 100, 1.5) : 0;

const childMax = () => children.value.reduce((m, c) => Math.max(m, c.sizeBytes), 0);

async function toggle() {
  if (props.node.isReparse || !props.node.hasChildren) return;
  expanded.value = !expanded.value;
  if (expanded.value && !loaded.value) {
    loading.value = true;
    try {
      children.value = await getChildren(props.node.id);
      loaded.value = true;
    } finally {
      loading.value = false;
    }
  }
}

const riskColor: Record<string, string> = {
  safe: "var(--color-success)",
  cost: "var(--color-warning)",
  caution: "var(--color-warning)",
};
</script>

<template>
  <div class="tree-row">
    <div
      class="row-main"
      :style="{ paddingLeft: `${depth * 20 + 12}px` }"
      :class="{ clickable: node.hasChildren && !node.isReparse }"
      @click="toggle"
    >
      <span class="caret" :class="{ open: expanded }">
        {{ node.hasChildren && !node.isReparse ? "▸" : "" }}
      </span>

      <div class="info">
        <div class="line1">
          <span class="name">{{ node.name }}</span>
          <span v-if="node.rule" class="tag" :style="{ color: riskColor[node.rule.risk] }">
            {{ node.rule.displayName }}
          </span>
          <span v-if="node.isReparse" class="tag moved">
            已迁移 → {{ node.reparseTarget ?? "其他位置" }}
          </span>
        </div>
        <div class="bar-track">
          <div class="bar-fill" :style="{ width: `${barPercent()}%` }"></div>
        </div>
        <p v-if="node.rule" class="explain">{{ node.rule.explain }}</p>
      </div>

      <div class="meta num">
        <span class="size">{{ fmtBytes(node.sizeBytes) }}</span>
        <span class="count">{{ fmtCount(node.fileCount) }} 个文件</span>
      </div>
    </div>

    <div v-if="expanded" class="children">
      <p v-if="loading" class="hint" :style="{ paddingLeft: `${(depth + 1) * 20 + 12}px` }">
        正在读取…
      </p>
      <p
        v-else-if="loaded && children.length === 0"
        class="hint"
        :style="{ paddingLeft: `${(depth + 1) * 20 + 12}px` }"
      >
        这个文件夹是空的
      </p>
      <TreeRow
        v-for="child in children"
        :key="child.id"
        :node="child"
        :sibling-max="childMax()"
        :depth="depth + 1"
      />
    </div>
  </div>
</template>

<style scoped>
.row-main {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 12px;
  border-radius: 8px;
}

.row-main.clickable {
  cursor: pointer;
}

.row-main.clickable:hover {
  background: var(--color-bg);
}

.caret {
  width: 14px;
  flex-shrink: 0;
  color: var(--color-text-secondary);
  transition: transform 0.15s;
  line-height: 1.6;
}

.caret.open {
  transform: rotate(90deg);
}

.info {
  flex: 1;
  min-width: 0;
}

.line1 {
  display: flex;
  align-items: center;
  gap: 8px;
}

.name {
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tag {
  flex-shrink: 0;
  font-size: var(--font-size-aux);
  padding: 1px 8px;
  border-radius: 6px;
  background: var(--color-bg);
}

.tag.moved {
  color: var(--color-text-secondary);
  max-width: 280px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bar-track {
  height: 6px;
  margin-top: 6px;
  border-radius: 3px;
  background: #eef0f2;
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  border-radius: 3px;
  background: var(--color-primary);
  transition: width 0.3s;
}

.explain {
  margin-top: 4px;
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}

.meta {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  min-width: 96px;
}

.size {
  font-weight: 600;
}

.count {
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}

.hint {
  padding: 6px 12px;
  font-size: var(--font-size-aux);
  color: var(--color-text-secondary);
}
</style>
