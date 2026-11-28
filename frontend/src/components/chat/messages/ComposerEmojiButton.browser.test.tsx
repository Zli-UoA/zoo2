import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRef, useState } from "react";
import { describe, expect, it, vi } from "vite-plus/test";
import { EmojiHistoryFixture, RECENT_EMOJIS_STORAGE_KEY } from "../../../test/emojiHistoryFixture";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ComposerEmojiButton } from "./ComposerEmojiButton";
import { HomeReactionPicker } from "../../home/HomeReactionPicker";
import { MfmContent } from "./MfmContent";

vi.mock("../../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: { apiBaseUrl: "http://localhost:8000" } }),
  useOptionalCollections: () => null,
}));
vi.mock("../../../lib/api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../lib/api/client")>()),
  createConfiguredApiClient: () => ({
    // /api/emojis returns a list; reaction-options returns an object.
    get: async (path: string) => (path === "/api/emojis" ? [] : { emojis: [] }),
  }),
}));

vi.mock("../../../hooks/useKitchenCatalog", () => ({
  useKitchenCatalog: () => ({
    catalog: {
      emojis: [
        ["1f600", "😀", "grinning face"],
        ["1f431", "🐱", "cat face"],
      ],
      dates: ["20230301"],
      pairs: { "0_1": 0 },
    },
    error: false,
    retry: () => {},
  }),
}));

function Editor({ composing = false }: { composing?: boolean }) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = useState("before replace after");
  return (
    <LocaleProvider>
      <textarea
        ref={textareaRef}
        aria-label="本文"
        value={value}
        onChange={(e) => setValue(e.target.value)}
      />
      <ComposerEmojiButton
        textareaRef={textareaRef}
        value={value}
        onChange={setValue}
        isComposing={() => composing}
      />
      <MfmContent content={value} emojis={{}} />
    </LocaleProvider>
  );
}

describe("Kitchen in message bodies", () => {
  it("replaces the saved selection with an MFM shortcode and renders the image", async () => {
    render(
      <EmojiHistoryFixture>
        <Editor />
      </EmojiHistoryFixture>,
    );
    const input = screen.getByRole("textbox", { name: "本文" }) as HTMLTextAreaElement;
    input.focus();
    input.setSelectionRange(7, 14);
    await userEvent.click(screen.getByRole("button", { name: "絵文字" }));
    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
    const image = screen.getByRole("dialog").querySelector("img")!;
    fireEvent.load(image);
    await userEvent.click(screen.getByRole("button", { name: "この組み合わせを追加" }));
    await waitFor(() => expect(input.value).toBe("before :kitchen_1f431_1f600: after"));
    expect(JSON.parse(localStorage.getItem(RECENT_EMOJIS_STORAGE_KEY)!)).toEqual([
      "kitchen:1f431_1f600",
    ]);
    const rendered = screen.getByRole("img", { name: "🐱 + 😀" });
    expect(rendered.getAttribute("src")).toContain("u1f600_u1f431.png");
    expect(input.selectionStart).toBe("before :kitchen_1f431_1f600:".length);
    await waitFor(() => expect(document.activeElement).toBe(input));
  });

  it("offers Kitchen on remote ActivityPub posts and keeps the reaction wire value", async () => {
    const select = vi.fn();
    const close = vi.fn();
    render(
      <LocaleProvider>
        <HomeReactionPicker
          postId="https://peer.test/note/1"
          remote
          onClose={close}
          onSelect={select}
        />
      </LocaleProvider>,
    );
    await userEvent.click(screen.getByRole("tab", { name: "Emoji Kitchen" }));
    fireEvent.load(screen.getByRole("dialog").querySelector("img")!);
    await userEvent.click(screen.getByRole("button", { name: "この組み合わせを追加" }));
    expect(select).toHaveBeenCalledWith({ reaction: "kitchen:1f431_1f600" });
    expect(close).toHaveBeenCalledOnce();
  });

  it("leaves IME composition untouched", async () => {
    render(<Editor composing />);
    await userEvent.click(screen.getByRole("button", { name: "絵文字" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("before replace after");
  });

  it("keeps inline code and plain content literal", () => {
    render(
      <MfmContent
        content={"`:kitchen_1f431_1f600:` <plain>:kitchen_1f431_1f600:</plain>"}
        emojis={{}}
      />,
    );
    expect(screen.queryByRole("img")).toBeNull();
  });
});
