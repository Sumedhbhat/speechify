import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const page = (name: string) => fileURLToPath(new URL(name, import.meta.url));

// Two windows: the main app and the floating dictation bar.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: "es2022",
    rolldownOptions: {
      input: { main: page("index.html"), bar: page("bar.html") },
    },
  },
});
