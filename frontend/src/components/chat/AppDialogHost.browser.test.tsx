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

it("opens the app dialog for zoo:open-app and zoo:message-actions", async () => {
  render(<AppDialogHost />);
  expect(screen.queryByTestId("app-dialog")).toBeNull();

  window.dispatchEvent(new CustomEvent("zoo:open-app", { detail: "app-1" }));
  await waitFor(() => expect(screen.getByTestId("app-dialog")).toHaveTextContent("dialog:app-1"));

  window.dispatchEvent(new CustomEvent("zoo:message-actions", { detail: "message-1" }));
  await waitFor(() =>
    expect(screen.getByTestId("app-dialog")).toHaveTextContent("dialog:message-1"),
  );

  expect(dialog.props).toEqual([
    { appId: "app-1", messageId: undefined },
    { appId: undefined, messageId: "message-1" },
  ]);
});
