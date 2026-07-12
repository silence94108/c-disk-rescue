<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { checkLocks, deleteOrphanProfile } from "../api";
import type { CleanableItem, CleanResult, OrphanProfile } from "../api/types";
import {
  checked,
  cleanItems,
  cleanPhase,
  cleanProgressBytes,
  cleanReport,
  dropOrphan,
  isDisabled,
  junkTotal,
  loadReportData,
  orphans,
  scanSummary,
  selectedBytes,
  selectedItems,
  startCleanSelected,
} from "../store";
import { fmtBytes } from "../utils/format";

const router = useRouter();

const errorMsg = ref("");
const doneResult = ref<CleanResult | null>(null);
const hasScan = computed(() => !!scanSummary.value);

interface Badge {
  text: string;
  cls: string;
}

function badge(item: CleanableItem): Badge {
  /* 引导型优先:它不参与执行,权限/占用状态都与它无关 */
  if (item.guideOnly)
    return { text: "本工具不代删,按上面的方法手动清理更安全", cls: "guide" };
  if (item.lockedBy.length > 0)
    return {
      text: `请先完全退出 ${item.lockedBy.join("、")}(它可能还在后台运行)`,
      cls: "gray",
    };
  if (item.needsAdmin && !cleanReport.value?.isElevated)
    return {
      text: "需要管理员权限:关掉本软件,右键它的图标选「以管理员身份运行」,再体检一次就能清理这项",
      cls: "gray",
    };
  if (item.risk === "safe") return { text: "放心删", cls: "safe" };
  if (item.risk === "cost") return { text: "删了有代价", cls: "cost" };
  return { text: "谨慎,想清楚再勾", cls: "caution" };
}

let lockTimer: number | undefined;
let polling = false;

onMounted(() => {
  if (scanSummary.value && !cleanReport.value) {
    loadReportData().catch(() => {
      errorMsg.value = "整理清理项时出了点问题,回概览再体检一次就好";
    });
  }
  // 占用状态轮询:用户退出被检出的软件后,行自动解锁亮起,无需手动刷新。
  // 只在存在锁定项时才发起复查,轻量接口只重测文件锁、不重新统计大小。
  lockTimer = window.setInterval(async () => {
    if (cleanPhase.value !== "idle" || polling) return;
    const locked = cleanItems.value.filter((i) => i.lockedBy.length > 0);
    if (locked.length === 0) return;
    polling = true;
    try {
      const statuses = await checkLocks(locked.map((i) => i.ruleId));
      for (const st of statuses) {
        const item = cleanItems.value.find((i) => i.ruleId === st.ruleId);
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

onUnmounted(() => {
  if (lockTimer !== undefined) window.clearInterval(lockTimer);
});

async function clean() {
  errorMsg.value = "";
  doneResult.value = null;
  try {
    const result = await startCleanSelected();
    if (result) doneResult.value = result;
  } catch {
    errorMsg.value = "清理没有完成,已删除的部分是安全的,再试一次就好";
  }
}

const cleanPercent = computed(() => {
  if (selectedBytes.value === 0) return 0;
  return Math.min(99, (cleanProgressBytes.value / selectedBytes.value) * 100);
});

/* ── 孤儿账户残留(融合方案 §2/§3):默认不勾、删除进回收站、独立于一键清理 ── */

const orphanChecked = ref<Record<string, boolean>>({});
const orphanAsk = ref(false);
const orphanBusy = ref(false);
const orphanDone = ref(0);
const orphanErr = ref("");

const orphanSelected = computed(() =>
  orphans.value.filter((o) => orphanChecked.value[o.path]),
);
const orphanSelectedBytes = computed(() =>
  orphanSelected.value.reduce((s, o) => s + o.sizeBytes, 0),
);

function orphanHintText(o: OrphanProfile): string {
  const src = o.hints.length ? `${o.hints.join("、")} 留下的缓存` : "来源软件未知的缓存残留";
  return `${src} · ${o.fileCount} 个文件,里面没有你的文档和照片`;
}

async function revealOrphan(o: OrphanProfile) {
  try {
    await revealItemInDir(o.path);
  } catch {
    /* 名字含损坏字符的目录路径打不开时,退而求其次带用户到 Users 目录 */
    try {
      await revealItemInDir(o.path.slice(0, o.path.lastIndexOf("\\")));
    } catch {
      /* 都打不开就算了,不打扰 */
    }
  }
}

async function deleteOrphans() {
  if (orphanBusy.value || orphanSelected.value.length === 0) return;
  orphanAsk.value = false;
  orphanBusy.value = true;
  orphanErr.value = "";
  for (const o of [...orphanSelected.value]) {
    try {
      await deleteOrphanProfile(o.path);
      orphanDone.value += o.sizeBytes;
      dropOrphan(o.path);
      delete orphanChecked.value[o.path];
    } catch (e) {
      orphanErr.value = String(e);
    }
  }
  orphanBusy.value = false;
}
</script>

<template>
  <div class="page">
    <!-- 未体检:引导回概览,不强制跳转(侧栏时代页面常驻) -->
    <section class="rcard guide-card" v-if="!hasScan">
      <span class="tile sm"><svg class="ic"><use href="#i-broom" /></svg></span>
      <div class="tx">
        <b>先做一次体检</b>
        <p>体检后这里会列出能安全清理的垃圾,每一项都有白话说明</p>
      </div>
      <button class="btn" @click="router.push('/')">去概览体检 ›</button>
    </section>

    <template v-else>
      <!-- 完成态:结果卡置顶,数字说话形成闭环(设计规范 §6) -->
      <section class="rcard result" v-if="doneResult">
        <p class="result-hero num">释放了 {{ fmtBytes(doneResult.freedBytes) }}!</p>
        <p class="result-sub num">
          共删除 {{ doneResult.deletedFiles }} 个文件<template
            v-if="doneResult.failedFiles > 0"
            >,另有 {{ doneResult.failedFiles }} 个文件正被使用,已自动跳过</template
          >
        </p>
        <p v-for="s in doneResult.skipped" :key="s.ruleId" class="result-skip">
          「{{ cleanItems.find((i) => i.ruleId === s.ruleId)?.displayName ?? s.ruleId }}」未清理:{{
            s.reason
          }}
        </p>
        <button class="btn" @click="router.push('/')">回概览看看效果</button>
      </section>

      <p class="error" v-if="errorMsg">{{ errorMsg }}</p>

      <section class="rcard">
        <div class="rhead2">
          <span class="tile sm"><svg class="ic"><use href="#i-broom" /></svg></span>
          <div class="tx">
            <b>发现可清理垃圾 <i class="num">{{ fmtBytes(junkTotal) }}</i></b>
            <p>已替你勾好「放心删」和「有代价」项,谨慎项需你手动勾选</p>
          </div>
          <button
            class="btn"
            v-if="cleanPhase === 'idle'"
            :disabled="selectedItems.length === 0"
            @click="clean"
          >
            一键清理（{{ fmtBytes(selectedBytes) }}）
          </button>
        </div>

        <!-- 清理中:进度可感知(>300ms 必须有反馈,设计规范 §6) -->
        <div class="cleaning" v-if="cleanPhase === 'cleaning'">
          <div class="bar">
            <div class="bar-fill" :style="{ width: cleanPercent + '%' }"></div>
          </div>
          <p class="cleaning-stat num">已释放 {{ fmtBytes(cleanProgressBytes) }}…</p>
        </div>

        <div class="rows" v-else>
          <p v-if="cleanItems.length === 0" class="empty">
            没有找到可清理的垃圾,你的 C 盘很干净。
          </p>
          <label
            v-for="item in cleanItems"
            :key="item.ruleId"
            class="row"
            :class="{ off: isDisabled(item) && !item.guideOnly }"
            :title="item.path"
          >
            <input
              type="checkbox"
              v-model="checked[item.ruleId]"
              :disabled="isDisabled(item)"
            />
            <div class="ib">
              <div class="nm">{{ item.displayName }}</div>
              <div class="ex">{{ item.explain }}</div>
              <span class="bg" :class="badge(item).cls">{{ badge(item).text }}</span>
            </div>
            <span class="sz num">{{ fmtBytes(item.sizeBytes) }}</span>
          </label>
        </div>
      </section>

      <!-- 孤儿账户残留(融合方案 §2/§3):被三条件识别的特定残骸 -->
      <section class="rcard orphan" v-if="orphans.length > 0 || orphanDone > 0">
        <div class="rhead2">
          <span class="tile sm warn"><svg class="ic"><use href="#i-alert" /></svg></span>
          <div class="tx">
            <b>发现疑似废弃的账户残留</b>
            <p>
              某些软件曾用错乱的名字在系统里留下缓存目录(名字显示为乱码是正常现象)。
              删不删由你决定;删除只进回收站,后悔了可以还原。
            </p>
          </div>
        </div>
        <p v-if="orphanDone > 0" class="orphan-done num">
          已移入回收站 {{ fmtBytes(orphanDone) }}(清空回收站后才真正腾出空间)
        </p>
        <p v-if="orphanErr" class="error">{{ orphanErr }}</p>
        <div class="rows">
          <label v-for="o in orphans" :key="o.path" class="row" :title="o.path">
            <input type="checkbox" v-model="orphanChecked[o.path]" :disabled="orphanBusy" />
            <div class="ib">
              <div class="nm">{{ o.name }}</div>
              <div class="ex">{{ orphanHintText(o) }}</div>
              <button class="reveal" @click.prevent="revealOrphan(o)">打开位置看看 ›</button>
            </div>
            <span class="sz num">{{ fmtBytes(o.sizeBytes) }}</span>
          </label>
        </div>
        <div class="orphan-ops" v-if="orphans.length > 0">
          <template v-if="orphanAsk">
            <span class="orphan-confirm">
              把选中的 {{ orphanSelected.length }} 个目录移进回收站(可反悔),确定吗?
            </span>
            <button class="op del" :disabled="orphanBusy" @click="deleteOrphans">删除</button>
            <button class="op" @click="orphanAsk = false">先不删</button>
          </template>
          <button
            v-else
            class="op del"
            :disabled="orphanSelected.length === 0 || orphanBusy"
            @click="orphanAsk = true"
          >
            {{
              orphanBusy
                ? "正在删除…"
                : `删除勾选的残留（${fmtBytes(orphanSelectedBytes)}）`
            }}
          </button>
        </div>
      </section>

      <div class="lastrow">
        <svg class="ic"><use href="#i-clock" /></svg>
        <span>只清内置白名单里的项,每次清理都有日志留痕,未知目录一律不碰</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
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

.rhead2 {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 16px;
}

.tile {
  border-radius: 16px;
  background: #e2f6ec;
  color: var(--color-action);
  display: grid;
  place-items: center;
  flex-shrink: 0;
}

.tile.sm {
  width: 46px;
  height: 46px;
  border-radius: 13px;
  font-size: 22px;
}

.tile.warn {
  background: #fdeeda;
  color: var(--pill-cost-fg);
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

.tx b i {
  font-style: normal;
  color: var(--color-action);
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

.rows {
  display: flex;
  flex-direction: column;
}

.row {
  display: flex;
  gap: 13px;
  padding: 15px 4px;
  border-top: 1px solid var(--color-line);
  align-items: flex-start;
  cursor: pointer;
}

.rows .row:first-child,
.rows .empty + .row {
  border-top: none;
}

.row.off {
  opacity: 0.6;
  cursor: not-allowed;
}

.row input {
  appearance: none;
  width: 22px;
  height: 22px;
  border-radius: 7px;
  border: 2px solid #b9c4d6;
  flex-shrink: 0;
  margin-top: 2px;
  cursor: pointer;
  transition: background 0.12s, border-color 0.12s;
}

.row input:checked {
  background: var(--color-primary)
    url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='white' stroke-width='3' stroke-linecap='round' stroke-linejoin='round'><polyline points='5 12.5 10 17.5 19 7'/></svg>")
    center / 14px no-repeat;
  border-color: var(--color-primary);
}

.row input:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.ib {
  flex: 1;
  min-width: 0;
}

.nm {
  font-size: 15.5px;
  font-weight: 700;
  color: var(--color-text);
}

.ex {
  font-size: 13.5px;
  color: var(--color-text-secondary);
  line-height: 1.6;
  margin: 2px 0 7px;
}

/* 安全三态药丸(F 稿:浅底深字、圆角 999) */
.bg {
  display: inline-flex;
  align-items: center;
  padding: 3px 11px;
  border-radius: 999px;
  font-size: 12.5px;
  font-weight: 600;
}

.bg.safe {
  background: var(--pill-safe-bg);
  color: var(--pill-safe-fg);
}

.bg.cost {
  background: var(--pill-cost-bg);
  color: var(--pill-cost-fg);
}

.bg.caution {
  background: var(--pill-caution-bg);
  color: var(--pill-caution-fg);
}

.bg.gray {
  background: #eef2f8;
  color: var(--color-text-secondary);
}

.bg.guide {
  background: #e4edfd;
  color: var(--color-primary);
}

.sz {
  font-size: 16px;
  font-weight: 900;
  flex-shrink: 0;
  color: var(--color-text);
}

.empty {
  padding: 24px 0 10px;
  text-align: center;
  color: var(--color-text-secondary);
}

.cleaning {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 18px 0 8px;
}

.bar {
  width: 100%;
  max-width: 480px;
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

.cleaning-stat {
  font-size: 16px;
  font-weight: 600;
}

.result {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 30px 26px;
}

.result-hero {
  font-size: 28px;
  font-weight: 900;
  color: var(--color-action);
}

.result-sub {
  color: var(--color-text-secondary);
}

.result-skip {
  font-size: 13px;
  color: var(--pill-cost-fg);
}

.result .btn {
  margin-top: 10px;
}

.error {
  padding: 10px 16px;
  border-radius: 10px;
  background: #fdeaea;
  color: var(--color-danger);
}

.orphan-done {
  padding: 8px 14px;
  margin-bottom: 8px;
  border-radius: 8px;
  background: #fdeeda;
  color: var(--pill-cost-fg);
}

.reveal {
  font-size: 12.5px;
  padding: 0;
  text-align: left;
  color: var(--color-primary);
}

.orphan-ops {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 10px;
}

.orphan-confirm {
  font-size: 13px;
  color: var(--pill-cost-fg);
}

.op {
  padding: 7px 14px;
  border-radius: 8px;
  border: 1px solid var(--color-line);
  background: var(--color-card);
}

.op.del {
  color: var(--pill-cost-fg);
  border-color: var(--pill-cost-fg);
}

.op:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.lastrow {
  background: rgba(255, 255, 255, 0.72);
  border: 1px solid #e6edf7;
  border-radius: 14px;
  padding: 13px 18px;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 14px;
  color: var(--color-text-secondary);
}
</style>
