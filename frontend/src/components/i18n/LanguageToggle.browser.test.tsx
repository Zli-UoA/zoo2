import "../../styles.css";
import { render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vite-plus/test";
import { cdp } from "vite-plus/test/browser/context";

import { LanguageToggle } from "./LanguageToggle";
import { LocaleProvider } from "./LocaleProvider";

afterEach(async () => {
  await cdp().send("Emulation.setTouchEmulationEnabled", { enabled: false });
});

it("gives the language icon button a 44px touch hit area", async () => {
  await cdp().send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 5 });
  render(
    <LocaleProvider>
      <LanguageToggle />
    </LocaleProvider>,
  );
  const button = screen.getByRole("button", { name: "言語を切り替える" });
  // アイコンの見た目は変えず、(hover: none) のときだけヒットエリアを広げる。
  expect(button.className).toContain("[@media(hover:none)]:min-h-11");
  expect(button.className).toContain("[@media(hover:none)]:min-w-11");
  const { width, height } = button.getBoundingClientRect();
  expect(width).toBeGreaterThanOrEqual(44);
  expect(height).toBeGreaterThanOrEqual(44);
});
