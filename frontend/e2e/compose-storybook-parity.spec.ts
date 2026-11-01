import { composeVisualColorThreshold } from "./compose-visual";
import { test, expect, type Page, type TestInfo } from "@playwright/test";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

type StoryEntry = { id: string; title: string; name: string; type: string };
type StoryMetadata = {
  globals: Record<string, unknown>;
  viewport: { width: number; height: number };
  args: string;
  hasPlay: boolean;
};
type ConsoleError = {
  implementation: "bootstrap" | "react" | "kotlin";
  text: string;
  location: { url: string; lineNumber: number; columnNumber: number };
};
type Result = {
  id: string;
  status: "passed" | "failed";
  error?: string;
  metadata?: StoryMetadata;
  consoleErrors?: ConsoleError[];
};
const buckets = 8;
const timeout = 90_000;
function describeError(error: unknown, depth = 0, seen = new Set<Error>()): string {
  if (depth >= 6) return "[cause depth limit]";
  if (!(error instanceof Error)) {
    try {
      return JSON.stringify(error) ?? "Unknown error";
    } catch {
      return "[unserializable thrown value]";
    }
  }
  if (seen.has(error)) return "[repeated error cause]";
  seen.add(error);
  const message = error.stack ?? error.message;
  const cause =
    error.cause === undefined ? "" : `\nCaused by: ${describeError(error.cause, depth + 1, seen)}`;
  const aggregate =
    error instanceof AggregateError
      ? error.errors
          .map((item: unknown) => `\nAggregate cause: ${describeError(item, depth + 1, seen)}`)
          .join("")
      : "";
  return message + cause + aggregate;
}
async function observe(page: Page) {
  await page.addInitScript(() => {
    const state = window as typeof window & {
      __PARITY_OPTIONS__?: { fixture?: { interactions?: unknown[] } };
      __PARITY_ERRORS__?: string[];
      __PARITY_FINISHED__?: { status: string };
      __STORYBOOK_ADDONS_CHANNEL__?: { on(name: string, listener: (value: unknown) => void): void };
    };
    state.__PARITY_ERRORS__ = [];
    window.addEventListener("message", (event) => {
      if (event.origin === location.origin && event.data?.type === "zoo-compose-catalog-options")
        state.__PARITY_OPTIONS__ = event.data.options;
    });
    const hook = setInterval(() => {
      const channel = state.__STORYBOOK_ADDONS_CHANNEL__;
      if (!channel) return;
      clearInterval(hook);
      channel.on("storyFinished", (value) => {
        state.__PARITY_FINISHED__ = value as { status: string };
      });
      for (const event of [
        "storyErrored",
        "storyThrewException",
        "playFunctionThrewException",
        "unhandledErrorsWhilePlaying",
        "storyMissing",
      ])
        channel.on(event, (value) =>
          state.__PARITY_ERRORS__!.push(`${event}: ${JSON.stringify(value)}`),
        );
    }, 1);
  });
  await page.clock.setFixedTime(new Date("2026-06-12T03:00:00Z"));
}
async function finished(page: Page) {
  try {
    await bounded(
      "Storybook play completion",
      page.waitForFunction(
        () => {
          const state = window as typeof window & {
            __PARITY_FINISHED__?: { status: string };
            __PARITY_ERRORS__?: string[];
            __STORYBOOK_PREVIEW__?: { currentRender?: { phase: string } };
          };
          return Boolean(
            state.__PARITY_ERRORS__?.length ||
            state.__PARITY_FINISHED__ ||
            state.__STORYBOOK_PREVIEW__?.currentRender?.phase === "finished",
          );
        },
        undefined,
        { timeout, polling: 50 },
      ),
      timeout + 5_000,
    );
  } catch (error) {
    const state = await bounded(
      "Storybook timeout diagnostic",
      page.evaluate(() => {
        const preview = (
          window as typeof window & {
            __STORYBOOK_PREVIEW__?: { currentRender?: { phase?: string; story?: { id?: string } } };
            __PARITY_ERRORS__?: string[];
            __PARITY_FINISHED__?: unknown;
          }
        ).__STORYBOOK_PREVIEW__;
        return {
          phase: preview?.currentRender?.phase,
          story: preview?.currentRender?.story?.id,
          title: document.title,
          visible: document.visibilityState,
          focused: document.hasFocus(),
          frames: Array.from(document.querySelectorAll("iframe")).map((frame) => ({
            src: frame.src,
            state: frame.dataset.composeCatalogState,
            rect: frame.getBoundingClientRect().toJSON(),
          })),
        };
      }),
    ).catch((diagnostic: unknown) => describeError(diagnostic));
    throw new Error(`Storybook did not finish: ${JSON.stringify(state)}`, { cause: error });
  }

  const errors = await bounded(
    "Storybook play errors",
    page.evaluate(() => {
      const state = window as typeof window & {
        __PARITY_ERRORS__?: string[];
        __PARITY_FINISHED__?: { status: string };
      };
      return [
        ...(state.__PARITY_ERRORS__ ?? []),
        ...(state.__PARITY_FINISHED__?.status === "error" ? ["Storybook play reported error"] : []),
      ];
    }),
  );
  expect(errors, "Storybook render/play must succeed").toEqual([]);
  await bounded("Storybook error display", expect(page.locator(".sb-errordisplay")).toBeHidden());
}
async function bounded<T>(phase: string, operation: Promise<T>, deadline = 30_000): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`Timed out after ${deadline}ms: ${phase}`)),
          deadline,
        );
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
async function fontsAndPaint(page: Page) {
  await bounded("focus measured page", page.bringToFront());
  for (const frame of page.frames()) {
    const phase = (stage: string) => `${frame.url()} ${stage}`;
    const progress = (stage: string) =>
      process.stdout.write(
        `[storybook-parity-paint] ${JSON.stringify({ frame: frame.url(), stage })}\n`,
      );
    progress("fonts-start");
    await bounded(
      phase("fonts.ready"),
      frame.evaluate(async () => {
        await document.fonts.ready;
      }),
    );
    progress("images-start");
    await bounded(
      phase("images"),
      frame.evaluate(async () => {
        await Promise.all(
          Array.from(document.images)
            .filter((image) => !image.complete)
            .map(
              (image) =>
                new Promise<void>((resolve, reject) => {
                  image.addEventListener("load", () => resolve(), { once: true });
                  image.addEventListener(
                    "error",
                    () => reject(new Error(`Image failed: ${image.currentSrc}`)),
                    { once: true },
                  );
                }),
            ),
        );
      }),
    );
    progress("native-image-decodes-start");
    await bounded(
      phase("native image decodes"),
      frame.waitForFunction(
        () =>
          !(globalThis as typeof globalThis & { __zooPendingImageDecodes?: number })
            .__zooPendingImageDecodes,
        undefined,
        { timeout: 30_000 },
      ),
    );
    progress("raf-start");
    await bounded(
      phase("two animation frames"),
      frame.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      ),
    );
    progress("ready");
  }
}

async function metadata(page: Page, storyId: string): Promise<StoryMetadata> {
  return bounded(
    "Storybook metadata",
    page.evaluate(async (id) => {
      type Context = {
        globals: Record<string, unknown>;
        args: Record<string, unknown>;
        parameters: {
          viewport?: { options?: Record<string, { styles: { width: string; height: string } }> };
        };
      };
      const preview = (
        window as typeof window & {
          __STORYBOOK_PREVIEW__: {
            storyStore: { loadStory(input: { storyId: string }): Promise<unknown> };
            getStoryContext(story: unknown): Context;
          };
        }
      ).__STORYBOOK_PREVIEW__;
      const story = await preview.storyStore.loadStory({ storyId: id });
      const context = preview.getStoryContext(story);
      const viewport = context.globals.viewport as { value?: string } | undefined;
      const styles = context.parameters.viewport?.options?.[viewport?.value ?? "desktop"]?.styles;
      const seen = new WeakSet<object>();
      const args = JSON.stringify(context.args, (key, value: unknown) => {
        if (key === "_owner") return undefined;
        if (typeof value === "function") return "[function]";
        if (value instanceof Set) return [...value];
        if (value && typeof value === "object") {
          if (seen.has(value)) return "[shared]";
          seen.add(value);
        }
        return value;
      });
      return {
        globals: context.globals,
        args,
        hasPlay: typeof (story as { playFunction?: unknown }).playFunction === "function",
        viewport: {
          width: parseInt(styles?.width ?? "1440", 10),
          height: parseInt(styles?.height ?? "900", 10),
        },
      };
    }, storyId),
  );
}
function url(entry: StoryEntry, implementation: string, globals?: Record<string, unknown>) {
  const values = [`implementation:${implementation}`];
  for (const key of ["theme", "locale"])
    if (typeof globals?.[key] === "string") values.push(`${key}:${globals[key]}`);
  return `/iframe.html?id=${encodeURIComponent(entry.id)}&viewMode=story&globals=${values.join(";")}`;
}
async function attach(page: Page, info: TestInfo, name: string) {
  const path = info.outputPath(`${name}.png`);
  await page.screenshot({ path, animations: "disabled", caret: "hide", timeout: 30_000 });
  await info.attach(name, { path, contentType: "image/png" });
}
// Every index story belongs to one bucket. --shard distributes all coverage without an allowlist.
for (let bucket = 0; bucket < buckets; bucket++)
  test(`all indexed stories parity bucket ${bucket + 1}/${buckets}`, async ({
    browser,
    request,
    baseURL,
  }, info) => {
    const response = await request.get("/index.json");
    expect(response.ok(), "Storybook index must load").toBe(true);
    const index = (await response.json()) as { entries: Record<string, StoryEntry> };
    const entries = Object.values(index.entries)
      .filter((entry) => entry.type === "story")
      .sort((a, b) => a.id.localeCompare(b.id));
    expect(entries.length, "Story index cannot be empty").toBeGreaterThan(0);
    const filterPattern = process.env.STORYBOOK_PARITY_FILTER?.trim() || undefined;
    const filter = filterPattern ? new RegExp(filterPattern) : undefined;
    const matched = filter
      ? entries.filter((entry) => filter.test(entry.title) || filter.test(entry.id))
      : entries;
    expect(matched.length, `No Story matched ${filterPattern ?? "the full index"}`).toBeGreaterThan(
      0,
    );
    const selected = matched.filter((_, position) => position % buckets === bucket);
    const results: Result[] = [];
    const report = () => ({
      totalIndexStories: entries.length,
      matchedStories: matched.length,
      filter: filterPattern ?? null,
      bucket,
      buckets,
      requestedStoryIds: selected.map((entry) => entry.id),
      complete: results.length === selected.length,
      results,
    });
    const persist = async () => {
      await mkdir(info.outputDir, { recursive: true });
      await writeFile(info.outputPath("results.json"), JSON.stringify(report(), null, 2));
    };
    const log = (event: Record<string, unknown>) =>
      process.stdout.write(
        `[storybook-parity] ${JSON.stringify({ bucket: bucket + 1, buckets, ...event })}\n`,
      );
    log({
      status: "start",
      filter: filterPattern ?? null,
      totalIndexStories: entries.length,
      matchedStories: matched.length,
      selectedStories: selected.length,
    });
    await persist();
    const context = await browser.newContext({
      baseURL,
      viewport: { width: 1440, height: 900 },
      locale: "ja-JP",
      timezoneId: "Asia/Tokyo",
      reducedMotion: "reduce",
    });
    // MSW unregisters itself when its last controlled client closes. Keep one inert,
    // same-origin client alive while the measured page navigates between implementations.
    const keeperPath = "/__storybook_parity_keepalive.html";
    const keeper = await context.newPage();
    await keeper.route(`**${keeperPath}`, (route) =>
      route.fulfill({
        status: 200,
        contentType: "text/html",
        body: "<!doctype html><html><head><title>Parity service worker keeper</title></head><body></body></html>",
      }),
    );
    await keeper.goto(keeperPath);
    let keeperControlled = false;
    const react = await context.newPage(),
      native = react;
    const errors: string[] = [];
    const consoleErrors: ConsoleError[] = [];
    let implementation: ConsoleError["implementation"] = "bootstrap";
    for (const page of new Set([react, native])) {
      await observe(page);
      await page.mouse.move(-100, -100);
      page.on("pageerror", (error) => errors.push(error.message));
      page.on("requestfailed", (request) =>
        consoleErrors.push({
          implementation,
          text: `Request failed: ${request.failure()?.errorText ?? "unknown"}`,
          location: { url: request.url(), lineNumber: 0, columnNumber: 0 },
        }),
      );
      page.on("console", (message) => {
        if (message.type() === "error") {
          if (/IllegalStateException: Node \d+ not found/.test(message.text()))
            errors.push(message.text());
          consoleErrors.push({
            implementation,
            text: message.text(),
            location: message.location(),
          });
        }
      });
    }
    try {
      for (const entry of selected) {
        if (react.isClosed())
          throw new Error(
            "Parity page closed before all indexed stories completed; see incomplete results.json",
          );
        await test.step(`${entry.title} / ${entry.name}`, async () => {
          const started = Date.now();
          log({ status: "story-start", id: entry.id, title: entry.title, name: entry.name });
          let details: StoryMetadata | undefined;
          let reference: Buffer | undefined;
          let resetNavigationComplete = false;
          try {
            log({ status: "stage", id: entry.id, stage: "reset-navigation" });
            await bounded("reset navigation", react.goto("about:blank", { timeout: 30_000 }));
            resetNavigationComplete = true;
            log({ status: "stage", id: entry.id, stage: "reset-cookies-permissions" });
            await bounded("clear cookies", context.clearCookies());
            await bounded("clear permissions", context.clearPermissions());
            const storage = await bounded("open storage session", context.newCDPSession(react));
            try {
              log({ status: "stage", id: entry.id, stage: "reset-origin-storage" });
              await bounded(
                "clear origin state",
                storage.send("Storage.clearDataForOrigin", {
                  origin: new URL(baseURL ?? "http://127.0.0.1:6006").origin,
                  // Each new document resets its MSW handlers; keep the registration
                  // so navigation cannot race a ServiceWorkerRegistration.update().
                  storageTypes: "local_storage,indexeddb,cache_storage",
                }),
              );
            } finally {
              await bounded("detach storage session", storage.detach());
            }
            await bounded("reset viewport", react.setViewportSize({ width: 1440, height: 900 }));
            errors.length = 0;
            consoleErrors.length = 0;
            implementation = "bootstrap";
            log({ status: "stage", id: entry.id, stage: "react-metadata-load" });
            await bounded("focus React page", react.bringToFront());
            await bounded("React metadata navigation", react.goto(url(entry, "react")));
            await bounded(
              "React metadata readiness",
              react.waitForFunction(
                () =>
                  Boolean(
                    (
                      window as typeof window & {
                        __STORYBOOK_PREVIEW__?: { storyStoreValue?: unknown };
                      }
                    ).__STORYBOOK_PREVIEW__?.storyStoreValue,
                  ),
                undefined,
                { timeout },
              ),
              timeout + 5_000,
            );
            log({ status: "stage", id: entry.id, stage: "react-metadata-ready" });
            details = await metadata(react, entry.id);
            if (!keeperControlled) {
              log({ status: "stage", id: entry.id, stage: "service-worker-keeper-control" });
              await bounded(
                "Service worker keeper control",
                keeper.waitForFunction(
                  () => navigator.serviceWorker.controller !== null,
                  undefined,
                  { timeout: 30_000, polling: 50 },
                ),
              );
              keeperControlled = true;
            }
            await bounded("story viewport", react.setViewportSize(details.viewport));
            await bounded(
              "clear localStorage",
              react.evaluate(() => localStorage.clear()),
            );
            await bounded("clear story cookies", context.clearCookies());
            errors.length = 0;
            consoleErrors.length = 0;
            implementation = "react";
            log({ status: "stage", id: entry.id, stage: "react-play" });
            await bounded("focus React page", react.bringToFront());
            await bounded("React navigation", react.goto(url(entry, "react", details.globals)));
            await finished(react);
            await fontsAndPaint(react);
            reference = await bounded(
              "React reference screenshot",
              react.screenshot({
                animations: "disabled",
                caret: "hide",
                timeout: 30_000,
              }),
            );
            const name = `${entry.id}.png`,
              referencePath = info.snapshotPath(name);
            await mkdir(dirname(referencePath), { recursive: true });
            // Only this execution's React render writes references, entirely inside outputDir.
            await writeFile(referencePath, reference);
            await bounded(
              "clear localStorage",
              react.evaluate(() => localStorage.clear()),
            );
            await bounded("clear story cookies", context.clearCookies());
            implementation = "kotlin";
            log({ status: "stage", id: entry.id, stage: "kotlin-load" });
            await bounded("focus Kotlin page", native.bringToFront());
            await bounded("Kotlin navigation", native.goto(url(entry, "kotlin", details.globals)));
            await bounded(
              "Kotlin story availability",
              expect(native.locator('[data-kotlin-story-status="unavailable"]')).toHaveCount(0),
            );
            await bounded(
              "Kotlin catalog readiness",
              expect(native.locator('[data-compose-catalog-state="ready"]')).toBeVisible({
                timeout,
              }),
              timeout + 5_000,
            );
            log({ status: "stage", id: entry.id, stage: "kotlin-ready" });
            await finished(native);
            log({ status: "stage", id: entry.id, stage: "kotlin-story-finished" });
            const frame = native
              .frames()
              .find((item) => new URL(item.url()).pathname === "/compose-catalog.html");
            if (!frame) throw new Error("Kotlin catalog frame missing");
            const interactions = await bounded(
              "Native fixture interactions",
              frame.evaluate(
                () =>
                  (
                    window as typeof window & {
                      __PARITY_OPTIONS__?: { fixture?: { interactions?: unknown[] } };
                    }
                  ).__PARITY_OPTIONS__?.fixture?.interactions ?? [],
              ),
            );
            log({
              status: "stage",
              id: entry.id,
              stage: "kotlin-play-verified",
              interactions: interactions.length,
            });
            if (details.hasPlay)
              expect(
                interactions.length,
                `React CSF play has no corresponding Native checks: ${entry.id}`,
              ).toBeGreaterThan(0);
            if (interactions.length > 0)
              expect(
                details.hasPlay,
                `Native fixture.interactions (${interactions.length}) have no corresponding React CSF playFunction: ${entry.id}`,
              ).toBe(true);
            await fontsAndPaint(native);
            expect(errors, "No browser runtime errors").toEqual([]);
            const nativeMetadata = await metadata(native, entry.id);
            expect(nativeMetadata.args, "Both implementations receive identical Story args").toBe(
              details.args,
            );
            expect(nativeMetadata.viewport).toEqual(details.viewport);
            // Cross-renderer font rasterization differs. Strict pixel comparisons are
            // opt-in; every run still executes both plays and captures both renders.
            if (process.env.STORYBOOK_PARITY_VISUAL === "strict") {
              await bounded(
                "Native stable screenshot comparison",
                expect(native).toHaveScreenshot(name, {
                  animations: "disabled",
                  caret: "hide",
                  timeout: 15_000,
                  threshold: composeVisualColorThreshold,
                  maxDiffPixels: 0,
                }),
                20_000,
              );
            } else {
              const actual = await bounded(
                "Kotlin review screenshot",
                native.screenshot({
                  path: info.outputPath(`${entry.id}-kotlin.png`),
                  animations: "disabled",
                  caret: "hide",
                  timeout: 30_000,
                }),
              );
              await info.attach(`${entry.id}-react`, { body: reference, contentType: "image/png" });
              await info.attach(`${entry.id}-kotlin`, { body: actual, contentType: "image/png" });
            }
            expect(errors, "No browser runtime errors after visual capture").toEqual([]);
            results.push({
              id: entry.id,
              status: "passed",
              metadata: details,
              consoleErrors: [...consoleErrors],
            });
          } catch (error) {
            const message =
              describeError(error) +
              (errors.length ? `\nBrowser runtime errors: ${JSON.stringify(errors)}` : "");
            results.push({
              id: entry.id,
              status: "failed",
              error: message,
              metadata: details,
              consoleErrors: [...consoleErrors],
            });
            await persist();
            log({ status: "failure-detected", id: entry.id, error: message.split("\n")[0] });
            if (consoleErrors.length)
              await info.attach(`${entry.id}-console-errors`, {
                body: JSON.stringify(consoleErrors, null, 2),
                contentType: "application/json",
              });
            await Promise.allSettled([
              reference
                ? info.attach(`${entry.id}-react`, { body: reference, contentType: "image/png" })
                : bounded("React failure screenshot", attach(react, info, `${entry.id}-react`)),
              bounded("native failure screenshot", attach(native, info, `${entry.id}-kotlin`)),
            ]);
            expect.soft(false, `${entry.id}: ${message}`).toBe(true);
            if (!resetNavigationComplete || react.isClosed())
              throw new Error(
                "Measured page cannot recover; remaining Story coverage is incomplete (results.json)",
                { cause: error },
              );
          } finally {
            await persist();
            const result = results.at(-1);
            log({
              status: result?.status ?? "failed",
              id: entry.id,
              durationMs: Date.now() - started,
              ...(result?.error ? { error: result.error.split("\n")[0] } : {}),
            });
          }
        });
      }
    } finally {
      await bounded("close parity context", context.close());
    }
    await persist();
    await info.attach("all-story-results", {
      body: JSON.stringify(report(), null, 2),
      contentType: "application/json",
    });
    log({
      status: "complete",
      passed: results.filter((result) => result.status === "passed").length,
      failed: results.filter((result) => result.status === "failed").length,
    });
    expect(results).toHaveLength(selected.length);
  });
