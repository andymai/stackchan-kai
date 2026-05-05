import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { viteSingleFile } from "vite-plugin-singlefile";

export default defineConfig({
  plugins: [solid(), viteSingleFile()],
  build: {
    target: "es2022",
    cssCodeSplit: false,
    assetsInlineLimit: 100_000_000,
    rollupOptions: {
      output: { inlineDynamicImports: true },
    },
  },
  server: {
    proxy: {
      "/state": "http://stackchan.local",
      "/health": "http://stackchan.local",
      "/settings": "http://stackchan.local",
      "/emotion": "http://stackchan.local",
      "/look-at": "http://stackchan.local",
      "/reset": "http://stackchan.local",
      "/speak": "http://stackchan.local",
      "/volume": "http://stackchan.local",
      "/mute": "http://stackchan.local",
      "/camera": "http://stackchan.local",
    },
  },
});
