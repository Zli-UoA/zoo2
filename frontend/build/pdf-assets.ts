import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";
import type { Plugin } from "vite-plus";

const require = createRequire(import.meta.url);
const packagePath = require.resolve("pdfjs-dist/package.json");
const { version } = require("pdfjs-dist/package.json") as { version: string };
const prefix = `assets/pdfjs/${version}/`;
const directories = ["cmaps", "wasm", "standard_fonts"];

/** Same versioned, same-origin decoder/font assets in the app, Storybook and APK. */
export function pdfAssets(): Plugin {
  let building = false;
  return {
    name: "pdf-preview-assets",
    configResolved(config) {
      building = config.command === "build";
    },
    async buildStart() {
      if (!building || this.environment?.config.consumer === "server") return;
      for (const directory of directories) {
        const root = path.join(path.dirname(packagePath), directory);
        for (const name of await readdir(root)) {
          // quickjs implements PDF scripting, which the preview does not enable.
          if (name.startsWith("quickjs")) continue;
          this.emitFile({
            type: "asset",
            fileName: `${prefix}${directory}/${name}`,
            source: await readFile(path.join(root, name)),
          });
        }
      }
    },
    configureServer(server) {
      server.middlewares.use(async (request, response, next) => {
        const pathname = new URL(request.url ?? "/", "http://localhost").pathname;
        const marker = `/${prefix}`;
        const start = pathname.indexOf(marker);
        if (start < 0) return next();
        const relative = pathname.slice(start + marker.length);
        const [directory, name, extra] = relative.split("/");
        if (
          !directories.includes(directory) ||
          !name ||
          name === "." ||
          name === ".." ||
          extra ||
          name.startsWith("quickjs")
        )
          return next();
        try {
          const bytes = await readFile(path.join(path.dirname(packagePath), directory, name));
          response.setHeader(
            "Content-Type",
            name.endsWith(".wasm")
              ? "application/wasm"
              : name.endsWith(".js")
                ? "text/javascript"
                : "application/octet-stream",
          );
          response.end(bytes);
        } catch {
          next();
        }
      });
    },
  };
}
