import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config.ts";

// Pure TypeScript tests: no DOM, so the node environment (no jsdom dependency).
export default mergeConfig(
  viteConfig,
  defineConfig({ test: { include: ["src/**/*.test.ts"], environment: "node" } }),
);
