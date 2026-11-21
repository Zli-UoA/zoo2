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
  // Hidden: nothing can be measured, so the field is fitted to 0.
  view.rerender(<Field hidden value={draft} />);
  view.rerender(<Field hidden={false} value={draft} />);
  const field = view.getByLabelText("draft");
  await waitFor(() => expect(heightOf(field)).toBeGreaterThanOrEqual(80));
});

it("fits the draft again when the width changes how it wraps", async () => {
  const draft = "word ".repeat(40);
  const view = render(<Field hidden={false} value={draft} width={600} />);
  const field = view.getByLabelText("draft");
  const wide = heightOf(field);
  view.rerender(<Field hidden={false} value={draft} width={120} />);
  await waitFor(() => expect(heightOf(field)).toBeGreaterThan(wide));
});

it("keeps a draft that fits unchanged when only the height changes", async () => {
  const view = render(<Field hidden={false} value={"1\n2"} />);
  const field = view.getByLabelText("draft");
  const before = heightOf(field);
  await new Promise((resolve) => setTimeout(resolve, 100));
  expect(heightOf(field)).toBe(before);
});
