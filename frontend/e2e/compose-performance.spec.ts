import { test, expect } from "@playwright/test";
import { fillControl, expectControlValue } from "./compose-control-input";

// Manual measurement suite: compare the same production Wasm builds with this
// identical fixture. No FPS threshold: shared CI machines do not have stable GPUs.
for (const implementation of ["react", "compose"] as const) {
  for (let sample = 0; sample < 3; sample++) {
    test(`${implementation}: performance sample ${sample + 1}`, async ({ page, request }, info) => {
      await request.post("https://127.0.0.1:4187/__reset");
      await request.post("https://127.0.0.1:4187/__seed", {
        data: {
          messages: Array.from({ length: 200 }, (_, i) => ({
            id: `perf-${i}`,
            content: `性能計測 ${i} 日本語の本文と English text`,
            created_at: new Date(Date.UTC(2026, 9, 31, 0, i)).toISOString(),
          })),
        },
      });
      await page.addInitScript(() => {
        performance.setResourceTimingBufferSize(5000);
        localStorage.setItem("zoo-locale", "ja");
        localStorage.setItem("zoo-theme", "light");
      });
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(`/channels/c1?frontend=${implementation}`);
      await expect(
        page.getByText("性能計測 199 日本語の本文と English text", { exact: true }),
      ).toBeVisible({ timeout: 90_000 });
      const readyMs = await page.evaluate(() => performance.now());
      const native = implementation === "compose" ? "kotlin" : "react";
      const composer = page.getByRole("textbox", { name: "メッセージを送信", exact: true });
      await fillControl(page, composer, "", native);
      await page.evaluate(() => {
        const frames: number[] = [];
        const state = { frames, stop: false };
        Object.assign(window, { __zooPerf: state });
        let last = performance.now();
        const frame = (now: number) => {
          frames.push(now - last);
          last = now;
          if (!state.stop) requestAnimationFrame(frame);
        };
        requestAnimationFrame(frame);
      });
      // Real keyboard events and wheel scrolling exercise input and virtualization.
      await page.keyboard.type("performance measurement input ".repeat(4), { delay: 15 });
      await page.mouse.move(950, 400);
      for (let i = 0; i < 16; i++) {
        await page.mouse.wheel(0, i < 8 ? -400 : 400);
        await page.waitForTimeout(50);
      }
      const metrics = await page.evaluate(() => {
        const state = (window as unknown as { __zooPerf: { frames: number[]; stop: boolean } })
          .__zooPerf;
        state.stop = true;
        const frames = state.frames.slice(1);
        const sorted = [...frames].sort((a, b) => a - b);
        return {
          frames: frames.length,
          fps: (1000 * frames.length) / frames.reduce((a, b) => a + b, 0),
          p95FrameMs: sorted[Math.ceil(sorted.length * 0.95) - 1],
          framesOver25ms: frames.filter((ms) => ms > 25).length,
          resourceBytes: performance
            .getEntriesByType("resource")
            .reduce((sum, entry) => sum + (entry as PerformanceResourceTiming).encodedBodySize, 0),
        };
      });
      const input =
        implementation === "compose" ? await composer.textContent() : await composer.inputValue();
      const result = {
        inputMatches: input === "performance measurement input ".repeat(4),
        inputLength: input?.length,

        label: process.env.COMPOSE_PERF_LABEL ?? "working-tree",
        implementation,
        sample,
        readyMs,
        ...metrics,
      };
      console.log(JSON.stringify(result));
      await info.attach("performance.json", {
        body: JSON.stringify(result, null, 2),
        contentType: "application/json",
      });
      expect(errors).toEqual([]);
      expect(metrics.frames).toBeGreaterThan(0);
      await expectControlValue(composer, "performance measurement input ".repeat(4), native);
    });
  }
}
