import { defineConfig } from "vite";

// FastAPI mounts dist/ under `/companion/`, so emit relative URLs.
// Dev server proxies /v1/* to the local sidecar (defaults to :8080).
export default defineConfig({
  base: "./",
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
  },
  server: {
    port: 5174,
    proxy: {
      "/v1": "http://localhost:8080",
    },
  },
});
