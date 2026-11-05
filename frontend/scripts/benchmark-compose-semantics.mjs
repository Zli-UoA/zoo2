// Isolated DOM boundary benchmark, not application FPS. Executes actual @JsFun
// bodies and mirrors the Kotlin dispatch. Run from frontend/ with Node.
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { chromium } from "@playwright/test";
const path = "frontend-compose/src/wasmJsMain/kotlin/zoo/WebSemantics.kt";
const baseline =
  process.argv[2] ?? execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const before = execFileSync("git", ["show", `${baseline}:${path}`], { encoding: "utf8" });
const after = await readFile(new URL(`../../${path}`, import.meta.url), "utf8");
function bodies(source) {
  return Object.fromEntries(
    [
      ...source.matchAll(
        /@JsFun\(\s*("""[\s\S]*?"""|"(?:\\.|[^"\\])*")\s*\)\s*private external fun (\w+)/g,
      ),
    ].map(([, literal, name]) => [
      name,
      literal.startsWith('"""') ? literal.slice(3, -3) : JSON.parse(literal),
    ]),
  );
}
const browser = await chromium.launch({
  ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}),
  args: ["--no-sandbox"],
});
const results = [];
try {
  for (let run = 0; run < 3; run++)
    for (const variant of run % 2 ? ["after", "before"] : ["before", "after"]) {
      const page = await browser.newPage();
      const result = await page.evaluate(
        async ({ source, variant }) => {
          const f = Object.fromEntries(
            // oxlint-disable-next-line no-eval -- Execute the checked-in @JsFun bodies, not a copied implementation.
            Object.entries(source).map(([name, body]) => [name, (0, eval)(`(${body})`)]),
          );
          const host = document.createElement("div");
          document.body.append(host);
          const root = host.attachShadow({ mode: "open" }),
            count = 150;
          root.innerHTML = Array.from(
            { length: count },
            (_, i) => `<div><div><button id="control-${i}">Control ${i}</button></div></div>`,
          ).join("");
          const ids = Array.from({ length: count }, (_, i) => `control-${i}`);
          const states = new Map();
          let lookups = 0,
            observer;
          for (const type of [Document, ShadowRoot]) {
            // oxlint-disable-next-line typescript/unbound-method -- The wrapper preserves this with apply().
            const original = type.prototype.querySelector;
            type.prototype.querySelector = function (...args) {
              lookups++;
              return original.apply(this, args);
            };
          }
          const apply = (id) => {
            const node = variant === "before" ? f.controlNode(id) : f.controlNode(observer, id);
            if (!node) return;
            for (const [name, value] of Object.entries({
              role: "button",
              "data-status": null,
              "aria-expanded": null,
              "aria-disabled": "false",
              "aria-selected": null,
              "aria-checked": null,
              "aria-pressed": String(states.get(id)),
            }))
              f.attribute(node, name, value);
          };
          const applyAll = () => [...states.keys()].forEach(apply);
          observer = f.observeControls(
            variant === "before" ? applyAll : (id) => (id ? apply(id) : applyAll()),
          );
          const update = (id) => {
            f.refreshObserver(observer);
            if (variant === "before") {
              applyAll();
              applyAll();
            } else {
              apply(id);
              apply(id);
            }
          };
          const start = performance.now();
          ids.forEach((id) => {
            states.set(id, false);
            update(id);
          });
          await new Promise(requestAnimationFrame);
          const initialMs = performance.now() - start,
            initialLookups = lookups;
          const frames = [],
            work = [];
          let last = await new Promise(requestAnimationFrame);
          for (let frame = 0; frame < 90; frame++) {
            const begin = performance.now();
            for (const id of ids) {
              states.set(id, frame % 2 === 0);
              update(id);
            }
            work.push(performance.now() - begin);
            const now = await new Promise(requestAnimationFrame);
            frames.push(now - last);
            last = now;
          }
          root.getElementById(ids[0]).setAttribute("role", "tab");
          root.getElementById(ids[1]).outerHTML = `<button id="${ids[1]}">Replacement</button>`;
          await new Promise(requestAnimationFrame);
          if (
            root.getElementById(ids[0]).getAttribute("role") !== "button" ||
            root.getElementById(ids[1]).getAttribute("role") !== "button"
          )
            throw new Error("Semantics restoration failed");
          f.closeObserver(observer);
          const p95 = (xs) => [...xs].sort((a, b) => a - b)[Math.ceil(xs.length * 0.95) - 1];
          return {
            initialMs,
            initialLookups,
            updateLookups: lookups - initialLookups,
            meanWorkMs: work.reduce((a, b) => a + b, 0) / work.length,
            p95FrameMs: p95(frames),
            fps: (1000 * frames.length) / frames.reduce((a, b) => a + b, 0),
            framesOver25ms: frames.filter((x) => x > 25).length,
          };
        },
        { source: bodies(variant === "before" ? before : after), variant },
      );
      results.push({ run, variant, ...result });
      await page.close();
    }
} finally {
  await browser.close();
}
const report = {
  kind: "isolated DOM boundary; Kotlin dispatch simulated; no Wasm/Canvas rendering",
  baseline,
  controls: 150,
  framesPerRun: 90,
  results,
};
const output = JSON.stringify(report, null, 2) + "\n";
if (process.argv[3]) await writeFile(process.argv[3], output);
process.stdout.write(output);
