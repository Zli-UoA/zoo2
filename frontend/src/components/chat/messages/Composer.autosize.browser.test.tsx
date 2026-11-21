import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vite-plus/test";

import "../../../styles.css";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import { Composer } from "./Composer";

function renderComposer(width: number) {
  const { container } = render(
    <LocaleProvider>
      <div data-testid="host" style={{ width }}>
        <Composer onSend={async () => true} />
      </div>
    </LocaleProvider>,
  );
  return {
    textarea: screen.getByRole("textbox"),
    host: container.querySelector<HTMLElement>('[data-testid="host"]')!,
  };
}

describe("Composer autosize", () => {
  it("recalculates the height when the available width changes without a value change", async () => {
    const { textarea, host } = renderComposer(320);
    await userEvent.type(textarea, "あ".repeat(60));

    const narrowHeight = textarea.getBoundingClientRect().height;
    expect(narrowHeight).toBeGreaterThan(50);

    host.style.width = "900px";
    const wideHeight = textarea.getBoundingClientRect().height;

    expect(wideHeight).toBeLessThan(narrowHeight);
  });

  it("caps the height and scrolls instead of growing without bound", async () => {
    const { textarea } = renderComposer(480);
    await userEvent.type(textarea, Array.from({ length: 30 }, (_, i) => `${i}行目`).join("\n"));

    expect(textarea.getBoundingClientRect().height).toBeLessThanOrEqual(200);
    expect(textarea.scrollHeight).toBeGreaterThan(textarea.clientHeight);
  });
});
