<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    percent: number;
    size?: number;
    strokeWidth?: number;
    color: string;
    breathing?: boolean;
  }>(),
  { size: 180, strokeWidth: 14, breathing: false },
);

const radius = computed(() => (props.size - props.strokeWidth) / 2);
const circumference = computed(() => 2 * Math.PI * radius.value);
const dash = computed(() => {
  const pct = Math.min(Math.max(props.percent, 0), 100);
  return `${(pct / 100) * circumference.value} ${circumference.value}`;
});
</script>

<template>
  <div class="donut" :class="{ breathing }" :style="{ width: `${size}px`, height: `${size}px` }">
    <svg :width="size" :height="size" :viewBox="`0 0 ${size} ${size}`">
      <circle
        :cx="size / 2"
        :cy="size / 2"
        :r="radius"
        fill="none"
        stroke="#e5e7eb"
        :stroke-width="strokeWidth"
      />
      <circle
        :cx="size / 2"
        :cy="size / 2"
        :r="radius"
        fill="none"
        :stroke="color"
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
