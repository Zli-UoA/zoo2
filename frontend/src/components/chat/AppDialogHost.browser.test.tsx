import { render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vite-plus/test";

import { AppDialogHost } from "./AppDialogHost";

const dialog = vi.hoisted(() => ({ props: [] as Array<{ appId?: string; messageId?: string }> }));

vi.mock("./AppsDialog", () => ({
  AppsDialog: ({
    initialId,
    messageId,
  }: {
    initialId?: string;
    messageId?: string;
    onClose: () => void;
  }) => {
    dialog.props.push({ appId: initialId, messageId });
    return <div data-testid="app-dialog">dialog:{initialId ?? messageId}</div>;
  },
}));

it.each([
  ["zoo:open-app", "app-1", { appId: "app-1", messageId: undefined }],
  ["zoo:message-actions", "message-1", { appId: undefined, messageId: "message-1" }],
] as const)("opens the dialog from %s", async (eventName, detail, expected) => {
  dialog.props = [];
  const { unmount } = render(<AppDialogHost />);
  expect(screen.queryByTestId("app-dialog")).toBeNull();

  window.dispatchEvent(new CustomEvent(eventName, { detail }));
  await waitFor(() =>
    expect(screen.getByTestId("app-dialog")).toHaveTextContent(`dialog:${detail}`),
  );
  expect(dialog.props).toEqual([expected]);
  unmount();
});
