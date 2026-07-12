<script setup lang="ts">
import { computed } from "vue";
import { RouterView, useRoute, useRouter } from "vue-router";

const route = useRoute();
const router = useRouter();

const NAV = [
  { path: "/", label: "概览", icon: "#i-home" },
  { path: "/clean", label: "垃圾清理", icon: "#i-broom" },
  { path: "/migrate", label: "搬家瘦身", icon: "#i-move" },
  { path: "/bigfiles", label: "大文件", icon: "#i-file" },
  { path: "/settings", label: "设置", icon: "#i-gear" },
];

/* 二级页高亮归属:空间地图属概览、已搬家属搬家瘦身(设计规范 §2) */
const activePath = computed(() => {
  if (route.path === "/map") return "/";
  if (route.path === "/moved") return "/migrate";
  return route.path;
});
</script>

<template>
  <!-- 线性图标库(F 稿:stroke 1.9 圆角端点,全项目经 <use> 复用,替代 emoji) -->
  <svg width="0" height="0" style="position: absolute" aria-hidden="true">
    <symbol id="i-hdd" viewBox="0 0 24 24"><rect x="3" y="6" width="18" height="12" rx="2"/><circle cx="16.5" cy="12" r="1.4" fill="currentColor" stroke="none"/><line x1="6.5" y1="12" x2="12.5" y2="12"/></symbol>
    <symbol id="i-trash" viewBox="0 0 24 24"><line x1="4" y1="7" x2="20" y2="7"/><path d="M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/><path d="M6.5 7l.9 12a1 1 0 0 0 1 1h7.2a1 1 0 0 0 1-1l.9-12"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></symbol>
    <symbol id="i-box" viewBox="0 0 24 24"><path d="M3.5 8.5h17v9a1 1 0 0 1-1 1h-15a1 1 0 0 1-1-1z"/><path d="M3.5 8.5l1.8-4h13.4l1.8 4"/><line x1="12" y1="4.5" x2="12" y2="8.5"/><line x1="9.5" y1="12" x2="14.5" y2="12"/></symbol>
    <symbol id="i-file" viewBox="0 0 24 24"><path d="M6.5 3.5h7l4 4v13a1 1 0 0 1-1 1h-10a1 1 0 0 1-1-1V4.5a1 1 0 0 1 1-1z"/><path d="M13.5 3.5v4h4"/></symbol>
    <symbol id="i-check" viewBox="0 0 24 24"><polyline points="5 12.5 10 17.5 19 7"/></symbol>
    <symbol id="i-cl" viewBox="0 0 24 24"><polyline points="14 6 8 12 14 18"/></symbol>
    <symbol id="i-alert" viewBox="0 0 24 24"><path d="M12 4l8.5 15h-17z"/><line x1="12" y1="10" x2="12" y2="14"/><line x1="12" y1="16.7" x2="12" y2="16.9"/></symbol>
    <symbol id="i-home" viewBox="0 0 24 24"><path d="M4 11l8-6 8 6"/><path d="M6 10v9h12v-9"/></symbol>
    <symbol id="i-broom" viewBox="0 0 24 24"><path d="M14 4l6 6"/><path d="M17 7l-7 7"/><path d="M10 14l-5 5 3 1 2-2"/><path d="M10 14l4 4"/></symbol>
    <symbol id="i-move" viewBox="0 0 24 24"><polyline points="14 8 18 12 14 16"/><line x1="18" y1="12" x2="8" y2="12"/><rect x="3" y="5" width="4" height="14" rx="1"/></symbol>
    <symbol id="i-gear" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1"/></symbol>
    <symbol id="i-pie" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><line x1="12" y1="12" x2="12" y2="3.5"/><line x1="12" y1="12" x2="18" y2="16"/></symbol>
    <symbol id="i-cd" viewBox="0 0 24 24"><polyline points="6 10 12 16 18 10"/></symbol>
    <symbol id="i-cr" viewBox="0 0 24 24"><polyline points="10 6 16 12 10 18"/></symbol>
    <symbol id="i-clock" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><polyline points="12 7.5 12 12 15 14"/></symbol>
    <symbol id="i-video" viewBox="0 0 24 24"><rect x="3.5" y="5.5" width="17" height="13" rx="2"/><path d="M10 9.5l5 2.5-5 2.5z"/></symbol>
    <symbol id="i-zip" viewBox="0 0 24 24"><rect x="4.5" y="4" width="15" height="16" rx="2"/><line x1="12" y1="4" x2="12" y2="8"/><line x1="12" y1="10" x2="12" y2="11.5"/><line x1="12" y1="13.5" x2="12" y2="15"/></symbol>
    <symbol id="i-installer" viewBox="0 0 24 24"><path d="M12 4v9"/><polyline points="8.5 9.5 12 13 15.5 9.5"/><path d="M5 16.5h14"/><path d="M6.5 16.5v3h11v-3"/></symbol>
    <symbol id="i-disc" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="2.6"/></symbol>
  </svg>

  <div class="shell">
    <aside class="side">
      <div class="brand">
        <span class="chip"><svg class="ic"><use href="#i-hdd" /></svg></span>
        <b>C盘救星</b>
      </div>
      <button
        v-for="n in NAV"
        :key="n.path"
        class="ni"
        :class="{ on: activePath === n.path }"
        @click="router.push(n.path)"
      >
        <svg class="ic"><use :href="n.icon" /></svg>{{ n.label }}
      </button>
      <!-- 等距硬盘插画(F 稿,仅氛围装饰不承载信息,设计规范 §4.5) -->
      <svg class="deco" width="146" height="118" viewBox="0 0 150 120" aria-hidden="true">
        <ellipse cx="72" cy="104" rx="46" ry="10" fill="#c9dcf6" opacity=".5"/>
        <path d="M28 62 L72 84 L72 102 L28 80 Z" fill="#b7d2f8"/>
        <path d="M116 62 L72 84 L72 102 L116 80 Z" fill="#9cc0f4"/>
        <path d="M28 62 L72 40 L116 62 L72 84 Z" fill="#e7f0fe"/>
        <ellipse cx="72" cy="62" rx="24" ry="12" fill="#fff"/>
        <ellipse cx="72" cy="62" rx="13" ry="6.5" fill="#dcebfe"/>
        <ellipse cx="72" cy="62" rx="5.5" ry="2.8" fill="#2563eb"/>
        <path d="M34 84 l9 4.5 v5 l-9 -4.5 Z" fill="#2f7cf0"/>
        <g transform="translate(8,14)"><path d="M0 6 L10 0 L20 6 L10 12 Z" fill="#e7f0fe"/><path d="M0 6 L10 12 V22 L0 16 Z" fill="#b7d2f8"/><path d="M20 6 L10 12 V22 L20 16 Z" fill="#9cc0f4"/></g>
        <g transform="translate(120,90) scale(.7)"><path d="M0 6 L10 0 L20 6 L10 12 Z" fill="#e7f0fe"/><path d="M0 6 L10 12 V22 L0 16 Z" fill="#b7d2f8"/><path d="M20 6 L10 12 V22 L20 16 Z" fill="#9cc0f4"/></g>
        <circle cx="128" cy="30" r="3.5" fill="#bcd7fb"/>
        <circle cx="20" cy="44" r="2.5" fill="#cfe3fc"/>
      </svg>
    </aside>
    <main class="main">
      <RouterView />
    </main>
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
}

.side {
  width: 196px;
  flex-shrink: 0;
  padding: 18px 14px 14px;
  display: flex;
  flex-direction: column;
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 8px 22px;
}

.chip {
  width: 34px;
  height: 34px;
  border-radius: 10px;
  background: var(--color-primary);
  color: #fff;
  display: grid;
  place-items: center;
  font-size: 17px;
  box-shadow: 0 6px 14px -6px rgba(37, 99, 235, 0.55);
  flex-shrink: 0;
}

.brand b {
  font-size: 16.5px;
  font-weight: 900;
  color: var(--color-text);
}

.ni {
  display: flex;
  align-items: center;
  gap: 11px;
  height: 46px;
  padding: 0 14px;
  border-radius: 12px;
  font-size: 15px;
  font-weight: 500;
  color: #465062;
  position: relative;
  margin-bottom: 4px;
  transition: background 0.15s;
  text-align: left;
}

.ni svg {
  font-size: 19px;
  color: #8a93a6;
}

.ni:hover {
  background: rgba(255, 255, 255, 0.75);
}

.ni.on {
  background: #fff;
  color: var(--color-primary);
  font-weight: 700;
  box-shadow: 0 8px 20px -10px rgba(31, 66, 135, 0.18);
}

.ni.on::before {
  content: "";
  position: absolute;
  left: 0;
  top: 11px;
  bottom: 11px;
  width: 4px;
  border-radius: 4px;
  background: var(--color-primary);
}

.ni.on svg {
  color: var(--color-primary);
}

.deco {
  margin-top: auto;
  align-self: center;
  pointer-events: none;
}

.main {
  flex: 1;
  min-width: 0;
  padding: 18px 20px 18px 8px;
  overflow-y: auto;
}
</style>
