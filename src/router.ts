import { createRouter, createWebHashHistory } from "vue-router";
import Home from "./pages/Home.vue";
import Report from "./pages/Report.vue";
import SpaceMap from "./pages/SpaceMap.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", component: Home },
    { path: "/report", component: Report },
    { path: "/map", component: SpaceMap },
  ],
});
