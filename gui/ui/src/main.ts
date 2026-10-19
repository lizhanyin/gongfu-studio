import { createApp } from "vue";
import App from "./App.vue";
import naive from "naive-ui";
import "./style.css";
import { api } from "./api";

// F12 切换开发者工具
window.addEventListener("keydown", (e) => {
  if (e.key === "F12") {
    e.preventDefault();
    api.toggleDevtools().catch(console.error);
  }
});

// 全局注册 naive-ui 组件; 模板中的 n-button/n-input 等 kebab-case 标签
// 依赖运行时 resolveComponent, 不注册会渲染成无样式的原生自定义元素.
createApp(App).use(naive).mount("#app");
