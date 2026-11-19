import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

/**
 * 开发：把 /api 与 /ws 代理到本地后端（默认 127.0.0.1:8791）。
 * 生产：后端同源托管 `dist/`，API 基址为空（同源）。
 *
 * 说明：本机环境下 esbuild 的 renderChunk 会在删除临时文件时被系统拒绝
 * （"remove ...: Access is denied."），因此关闭 esbuild 压栈/降级：
 * `target: "esnext"` + `minify: false`。本地终端不需要压缩产物。
 */
const backend = process.env.ARBX_API?.trim() || "http://127.0.0.1:8791";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: false,
    proxy: {
      "/api": { target: backend, changeOrigin: false },
      "/ws": { target: backend.replace(/^http/, "ws"), ws: true },
    },
  },
  build: {
    outDir: "dist",
    target: "esnext",
    minify: false,
    sourcemap: false,
  },
});
