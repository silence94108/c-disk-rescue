import { createApp } from "vue";
import App from "./App.vue";
import { router } from "./router";
import { recoverPendingMigration } from "./api";
import { recoverNotice } from "./store";
import "./styles/tokens.css";

createApp(App).use(router).mount("#app");

// 启动时检查未完成的搬家事务:断电/强关后自动恢复原状并告知(需求文档 §7)
recoverPendingMigration()
  .then((msg) => {
    if (msg) recoverNotice.value = msg;
  })
  .catch(() => {});
