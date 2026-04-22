import path from "node:path";

import { defineConfig } from "vite";

export default defineConfig({
  build: {
    target: "node20",
    sourcemap: true,
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
});
