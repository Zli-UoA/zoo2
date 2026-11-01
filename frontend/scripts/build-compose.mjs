import { cp, mkdir, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const project = fileURLToPath(new URL("../../frontend-compose/", import.meta.url));
const result = spawnSync(
  process.platform === "win32" ? "gradlew.bat" : "./gradlew",
  [
    "wasmJsBrowserDistribution",
    ...(process.env.ZOO_COMPOSE_SYSTEM_NODE === "true" ? ["-Pzoo.systemNode=true"] : []),
  ],
  {
    cwd: project,
    stdio: "inherit",
    shell: process.platform === "win32",
    // Binaryen otherwise sizes its worker pool from host CPUs, even in a smaller container.
    env: { ...process.env, BINARYEN_CORES: process.env.BINARYEN_CORES ?? "1" },
  },
);
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);

const destination = fileURLToPath(new URL("../public/compose/", import.meta.url));
// Replace only this generated directory, and only after a successful build.
await rm(destination, { recursive: true, force: true });
await mkdir(destination, { recursive: true });
await cp(
  new URL("../../frontend-compose/build/dist/wasmJs/productionExecutable/", import.meta.url),
  destination,
  { recursive: true },
);
