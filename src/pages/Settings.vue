<script setup lang="ts">
import { onMounted, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { appDataDir } from "@tauri-apps/api/path";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

const version = ref("");
const dataDir = ref("");
const notice = ref("");

onMounted(async () => {
  version.value = await getVersion().catch(() => "");
  dataDir.value = await appDataDir().catch(() => "");
});

/** 清理与搬家日志都落在应用数据目录(cleaner/migrator 的 data_file) */
async function openLogs() {
  if (!dataDir.value) return;
  try {
    await revealItemInDir(dataDir.value);
  } catch {
    notice.value = "没能打开日志文件夹";
  }
}
</script>

<template>
  <div class="page">
    <div class="phead">
      <div class="ptitle">设置</div>
    </div>

    <p v-if="notice" class="notice">{{ notice }}</p>

    <section class="rcard">
      <div class="sec-title">搬家</div>
      <div class="line">
        <div class="ib">
          <div class="nm">搬家目标位置</div>
          <div class="ex">
            搬走的目录统一放进目标盘的 AppDataMove 文件夹(如 D:\AppDataMove\),
            目标盘在搬家时选择。首版暂不支持自定义文件夹名。
          </div>
        </div>
      </div>
    </section>

    <section class="rcard">
      <div class="sec-title">数据与日志</div>
      <div class="line">
        <div class="ib">
          <div class="nm">操作日志</div>
          <div class="ex">每次清理和搬家都有记录留痕,出问题可以追溯。</div>
        </div>
        <button class="op" @click="openLogs">打开日志文件夹</button>
      </div>
    </section>

    <section class="rcard">
      <div class="sec-title">关于</div>
      <div class="line">
        <div class="ib">
          <div class="nm">C盘救星 <span class="num" v-if="version">v{{ version }}</span></div>
          <div class="ex">
            免费、无捆绑、不常驻后台,用完即走。清理规则全部内置明文,
            只碰白名单里的目录;搬家步步可撤销,未知目录一律不动。
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.phead {
  padding: 2px 4px 0;
}

.ptitle {
  font-size: 21px;
  font-weight: 900;
  color: var(--color-text);
}

.notice {
  padding: 10px 14px;
  border-radius: 10px;
  background: var(--pill-cost-bg);
  color: var(--pill-cost-fg);
}

.rcard {
  background: var(--color-card);
  border-radius: var(--radius-card);
  padding: 20px 26px;
  box-shadow: var(--shadow-card);
}

.sec-title {
  font-size: 13px;
  font-weight: 700;
  color: var(--color-text-secondary);
  margin-bottom: 10px;
}

.line {
  display: flex;
  align-items: center;
  gap: 16px;
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

.nm span {
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text-secondary);
  margin-left: 6px;
}

.ex {
  font-size: 13.5px;
  color: var(--color-text-secondary);
  line-height: 1.6;
  margin-top: 2px;
}

.op {
  padding: 8px 16px;
  border-radius: 8px;
  border: 1px solid var(--color-line);
  background: var(--color-card);
  color: var(--color-text);
  flex-shrink: 0;
}

.op:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
}
</style>
