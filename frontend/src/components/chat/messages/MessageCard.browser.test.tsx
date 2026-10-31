import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { MessageCard, type CardData } from "./MessageCard";
import "../../../styles.css";

const card: CardData = {
  title: "Deployment",
  description: "",
  url: "https://example.com/deploy",
  source: "GitHub",
  context: "",
  author: "Deployment bot",
  badge: "",
  fields: [
    { title: "Status", value: "Completed", short: true },
    { title: "Commit", value: "a-long-commit-description", short: true, code: true },
    { title: "Workflow", value: "Release" },
  ],
};
function renderCard(data: CardData, width: number, attachment = true) {
  return render(
    <LocaleProvider>
      <div style={{ width }}>
        <MessageCard attachment={attachment} card={data} />
      </div>
    </LocaleProvider>,
  );
}
it.each([
  [300, 1],
  [700, 2],
])("lays out card fields within a %ipx container", (width, columns) => {
  const view = renderCard(card, width);
  const fields = view.container.querySelector("dl")!;
  expect(getComputedStyle(fields).gridTemplateColumns.split(" ")).toHaveLength(columns);
  expect(fields.scrollWidth).toBeLessThanOrEqual(width);
});

it.each([false, true])(
  "expands only a long description without an inner scroll area (attachment=%s)",
  async (attachment) => {
    renderCard(
      { ...card, description: Array.from({ length: 20 }, (_, i) => `Line ${i + 1}`).join("\n") },
      320,
      attachment,
    );
    const toggle = await screen.findByRole("button", { name: "もっと見る" });
    const body = document.getElementById(toggle.getAttribute("aria-controls")!)!;
    const collapsedHeight = body.getBoundingClientRect().height;
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(body.scrollHeight).toBeGreaterThan(body.clientHeight);
    expect(getComputedStyle(body).overflowY).toBe("clip");
    body.scrollTop = 50;
    expect(body.scrollTop).toBe(0);
    expect(screen.getByText("Completed")).toBeVisible();
    expect(screen.getByText("Deployment bot")).toBeVisible();
    expect(body.contains(screen.getByText("Completed"))).toBe(false);
    expect(body.contains(screen.getByText("Deployment bot"))).toBe(false);
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(body.getBoundingClientRect().height).toBeGreaterThan(collapsedHeight);
    expect(body.scrollHeight).toBeLessThanOrEqual(body.clientHeight + 1);
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(body.getBoundingClientRect().height).toBe(collapsedHeight);
  },
);

it("only offers expansion when text overflows, including after a container resize", async () => {
  const data = { ...card, description: "A release description with wrapping text. ".repeat(12) };
  const view = renderCard(data, 700);
  expect(screen.queryByRole("button", { name: "もっと見る" })).toBeNull();
  view.rerender(
    <LocaleProvider>
      <div style={{ width: 180 }}>
        <MessageCard card={data} />
      </div>
    </LocaleProvider>,
  );
  const toggle = await screen.findByRole("button", { name: "もっと見る" });
  fireEvent.click(toggle);
  view.rerender(
    <LocaleProvider>
      <div style={{ width: 180 }}>
        <MessageCard card={{ ...card, description: "Short text" }} />
      </div>
    </LocaleProvider>,
  );
  await waitFor(() => expect(screen.queryByRole("button", { name: "折りたたむ" })).toBeNull());
  expect(screen.getByText("Short text")).toBeVisible();
});
