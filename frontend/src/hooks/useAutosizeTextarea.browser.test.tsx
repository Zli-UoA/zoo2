import { render, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { expect, it } from "vite-plus/test";

import { useAutosizeTextarea } from "./useAutosizeTextarea";

function Field({ hidden, value, width = 200 }: { hidden: boolean; value: string; width?: number }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useAutosizeTextarea(ref, value, 160);
  return (
    <div hidden={hidden}>
      <textarea
        ref={ref}
        aria-label="draft"
        readOnly
        rows={1}
        value={value}
        style={{ width, lineHeight: "20px", padding: 0, border: 0, boxSizing: "content-box" }}
      />
    </div>
  );
}

const heightOf = (element: HTMLElement) => element.getBoundingClientRect().height;

it("fits a draft that was set while the field was hidden once it is shown again", async () => {
  const draft = "1\n2\n3\n4";
  const view = render(<Field hidden value="" />);
  // Hidden: nothing can be measured. The field is left alone, not fitted to 0.
  view.rerender(<Field hidden value={draft} />);
  expect(view.getByLabelText("draft").style.height).not.toBe("0px");
  view.rerender(<Field hidden={false} value={draft} />);
  const field = view.getByLabelText("draft");
  await waitFor(() => expect(heightOf(field)).toBeGreaterThanOrEqual(80));
});

it("grows with the draft while it is shown", async () => {
  const view = render(<Field hidden={false} value="1" />);
  const field = view.getByLabelText("draft");
  const one = heightOf(field);
  view.rerender(<Field hidden={false} value={"1\n2\n3\n4"} />);
  await waitFor(() => expect(heightOf(field)).toBeGreaterThan(one));
  view.rerender(<Field hidden={false} value="1" />);
  await waitFor(() => expect(heightOf(field)).toBe(one));
});

// What matters is that the draft is not cut off, not an exact height (fonts differ): the field is as
// tall as its content, or it is at the cap and scrolls.
const clipped = (field: HTMLElement) =>
  field.scrollHeight > field.clientHeight + 1 && getComputedStyle(field).overflowY !== "auto";

it("fits the same draft again when a narrower field wraps it onto more lines", async () => {
  const draft = "word ".repeat(60);
  const view = render(<Field hidden={false} value={draft} width={800} />);
  const field = view.getByLabelText("draft");
  await waitFor(() => expect(clipped(field)).toBe(false));
  const wide = heightOf(field);
  view.rerender(<Field hidden={false} value={draft} width={120} />);
  await waitFor(() => expect(heightOf(field)).toBeGreaterThan(wide), { timeout: 3000 });
  expect(clipped(field)).toBe(false);
});

it("keeps a draft that fits unchanged across renders", async () => {
  const view = render(<Field hidden={false} value={"1\n2"} />);
  const field = view.getByLabelText("draft");
  const before = heightOf(field);
  view.rerender(<Field hidden={false} value={"1\n2"} />);
  await new Promise((resolve) => setTimeout(resolve, 100));
  expect(heightOf(field)).toBe(before);
});
