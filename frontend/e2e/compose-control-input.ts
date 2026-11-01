import { expect, type Locator, type Page } from "@playwright/test";
export type Implementation = "react" | "kotlin";
export async function fillControl(
  page: Page,
  field: Locator,
  value: string,
  implementation: Implementation,
) {
  if (implementation === "react") return field.fill(value);
  const previous = (await field.textContent()) ?? "";
  await field.click({ force: true });
  const owner = () =>
    field.evaluate((element) => {
      let active = element.ownerDocument.activeElement;
      while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
      return active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement
        ? { value: active.value, start: active.selectionStart, end: active.selectionEnd }
        : null;
    });
  await expect.poll(async () => (await owner())?.value).toBe(previous);
  await page.keyboard.press("ControlOrMeta+A");
  await expect.poll(owner).toEqual({ value: previous, start: 0, end: previous.length });
  await page.keyboard.press("Backspace");
  await expect(field).toHaveText("");
  if (value) await page.keyboard.insertText(value);
  await expect(field).toHaveText(value);
}
export async function expectControlValue(
  field: Locator,
  expected: string,
  implementation: Implementation,
) {
  if (implementation === "react") await expect(field).toHaveValue(expected);
  else await expect(field).toHaveText(expected);
}
