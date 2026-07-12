<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    percent: number;
    size?: number;
    strokeWidth?: number;
    color: string;
    /** 渐变描边(F 稿容量环 #7DB4FF→#2563EB),设置后优先于 color */
    gradient?: [string, string];
    trackColor?: string;
    breathing?: boolean;
  }>(),
  { size: 180, strokeWidth: 14, trackColor: "#e8eefb", breathing: false },
);

/* 同页多实例时渐变 id 不能撞 */
const uid = `dg-${Math.random().toString(36).slice(2, 8)}`;

const radius = computed(() => (props.size - props.strokeWidth) / 2);
const circumference = computed(() => 2 * Math.PI * radius.value);
const dash = computed(() => {
  const pct = Math.min(Math.max(props.percent, 0), 100);
  return `${(pct / 100) * circumference.value} ${circumference.value}`;
});
const stroke = computed(() => (props.gradient ? `url(#${uid})` : props.color));
</script>

<template>
  <div class="donut" :class="{ breathing }" :style="{ width: `${size}px`, height: `${size}px` }">
    <svg :width="size" :height="size" :viewBox="`0 0 ${size} ${size}`">
      <defs v-if="gradient">
        <linearGradient :id="uid" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" :stop-color="gradient[0]" />
          <stop offset="1" :stop-color="gradient[1]" />
        </linearGradient>
      </defs>
      <circle
        :cx="size / 2"
        :cy="size / 2"
        :r="radius"
        fill="none"
        :stroke="trackColor"
        :stroke-width="strokeWidth"
      />
      <circle
        :cx="size / 2"
        :cy="size / 2"
        :r="radius"
        fill="none"
        :stroke="stroke"
        :stroke-width="strokeWidth"
        stroke-linecap="round"
        :stroke-dasharray="dash"
        :transform="`rotate(-90 ${size / 2} ${size / 2})`"
        class="arc"
      />
    </svg>
    <div class="center">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.donut {
  position: relative;
  display: inline-block;
}

.arc {
  transition: stroke-dasharray 0.6s ease, stroke 0.3s ease;
}

.center {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
}

/* C 盘用量 >90% 的爆红呼吸动效(设计规范 3.1) */
.breathing {
  animation: breathe 2s ease-in-out infinite;
}

@keyframes breathe {
  0%,
  100% {
    filter: none;
  }
  50% {
    filter: drop-shadow(0 0 10px rgba(220, 38, 38, 0.55));
  }
}
</style>
