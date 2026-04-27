import path from "node:path";

import { defineConfig } from "vitest/config";

export default defineConfig({
  clearScreen: false,
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "node",
    include: ["electron/**/*.test.ts"],
  },
});
