import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // es2022：支持 fmath wasm wrapper（gfm.ts）的顶层 await init
    target: "es2022",
  },
});
