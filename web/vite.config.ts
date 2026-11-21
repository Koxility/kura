import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  resolve: {
    // A copy of ascii.rest's source, newer than its npm package; see vendor/ascii.rest.
    alias: {
      "ascii.rest": decodeURIComponent(
        new URL("./vendor/ascii.rest/src/index.ts", import.meta.url).pathname,
      ),
    },
  },
  server: { proxy: { "/api": "http://localhost:8787" } },
});
