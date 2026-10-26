import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";

import type { CollectionsBundle } from "./collections";
import { CollectionsProvider, useCollections } from "./collections-context";

const mocks = vi.hoisted(() => ({ getCollections: vi.fn() }));

vi.mock("./collections", () => ({
  getCollections: mocks.getCollections,
}));

const fallback = { marker: "fallback" } as unknown as CollectionsBundle;
const injected = { marker: "injected" } as unknown as CollectionsBundle;

function Probe() {
  const bundle = useCollections() as unknown as { marker: string };
  return <span>{bundle.marker}</span>;
}

describe("useCollections", () => {
  beforeEach(() => {
    mocks.getCollections.mockReset();
    mocks.getCollections.mockReturnValue(fallback);
  });

  it("uses the existing singleton when no provider is present", () => {
    render(<Probe />);

    expect(screen.getByText("fallback")).toBeTruthy();
    expect(mocks.getCollections).toHaveBeenCalledTimes(1);
  });

  it("uses the injected bundle when a provider is present", () => {
    render(
      <CollectionsProvider value={injected}>
        <Probe />
      </CollectionsProvider>,
    );

    expect(screen.getByText("injected")).toBeTruthy();
    expect(mocks.getCollections).not.toHaveBeenCalled();
  });
});
