import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement } from "react";
import { I18nextProvider } from "react-i18next";
import { expect, it, vi } from "vite-plus/test";

import "../../styles.css";

import { createI18n } from "../../lib/i18n";
import { BubbleChatScreen } from "./BubbleChatScreen";

vi.mock("../../hooks/useBubbleChat", async () => {
  const { useState } = await import("react");
  const entries: never[] = [];
  return {
    useBubbleChat: () => {
      const [content, setContent] = useState("");
      return {
        entries,
        content,
        isSharing: true,
        setContent,
        clearContent: () => setContent(""),
      };
    },
  };
});

vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "user-me", isLoading: false }),
}));

vi.mock("../../hooks/useRealtime", () => ({
  useUsers: () => ({ data: [] }),
}));

it("grows the bubble chat input up to its 180px cap and then scrolls", async () => {
  render(
    createElement(I18nextProvider, { i18n: createI18n("ja") }, createElement(BubbleChatScreen)),
  );
  const textarea = screen.getByRole("textbox");
  textarea.style.width = "200px";
  await userEvent.type(textarea, "あ".repeat(400));

  const height = textarea.getBoundingClientRect().height;
  expect(height).toBeGreaterThanOrEqual(170);
  expect(height).toBeLessThanOrEqual(180);
  expect(textarea.scrollHeight).toBeGreaterThan(textarea.clientHeight);
});
