import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  server: { proxy: { "/api": {
    target: "http://127.0.0.1:4290",
    ws: true,
    configure(proxy) {
      proxy.on("proxyReq", (proxyReq, request) => {
        if (request.headers.host) proxyReq.setHeader("host", request.headers.host);
      });
    },
  } } },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("/node_modules/@xterm/")) return "xterm";
          if (/\/node_modules\/(react|react-dom|scheduler)\//.test(id)) return "react";
          if (id.includes("/node_modules/@radix-ui/") || id.includes("/node_modules/radix-ui/")) return "radix";
        },
      },
    },
  },
  test: { environment: "jsdom" },
});
