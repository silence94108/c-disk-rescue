import { createRouter, createWebHashHistory } from "vue-router";
import Home from "./pages/Home.vue";
import Clean from "./pages/Clean.vue";
import SpaceMap from "./pages/SpaceMap.vue";
import Migrate from "./pages/Migrate.vue";
import Moved from "./pages/Moved.vue";
import BigFiles from "./pages/BigFiles.vue";
import Settings from "./pages/Settings.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", component: Home },
    { path: "/clean", component: Clean },
    { path: "/migrate", component: Migrate },
    { path: "/bigfiles", component: BigFiles },
    { path: "/settings", component: Settings },
    // 二级页:空间地图从概览进入,已搬家从搬家瘦身进入(设计规范 §2)
    { path: "/map", component: SpaceMap },
    { path: "/moved", component: Moved },
    { path: "/report", redirect: "/clean" },
  ],
});
