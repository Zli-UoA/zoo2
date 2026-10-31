import { rawCopyMessage } from "./message-content";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { LocaleProvider } from "../../src/components/i18n/LocaleProvider";
import { useVisualViewport } from "../../src/components/layout/useVisualViewport";
import { MessageList } from "../../src/components/chat/messages/MessageList";
import { Composer } from "../../src/components/chat/messages/Composer";
import "../../src/styles.css";

function Layout() {
  const ref = useVisualViewport();
  const [messages, setMessages] = useState(() =>
    Array.from({ length: 40 }, (_, i) => ({
      id: String(i),
      user_id: "me",
      display_name: "Alice",
      parent_id: null,
      content:
        i === 37 && new URLSearchParams(window.location.search).has("raw-copy")
          ? rawCopyMessage
          : i === 39
            ? "Message 39"
            : `Message ${i}\n` + "Original line\n".repeat((i % 4) + 1),
      created_at: "2026-01-02 00:00:00.000",
    })),
  );
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const noop = () => {};
  const prependOlder = () => {
    setMessages((current) => [
      ...Array.from({ length: 10 }, (_, i) => ({
        id: `older-${i}-${current.length}`,
        user_id: "me",
        display_name: "Alice",
        parent_id: null,
        content: `Older ${i}\n` + "Older line\n".repeat((i % 3) + 1),
        created_at: "2026-01-01 00:00:00.000",
      })),
      ...current,
    ]);
  };
  const appendNew = () => {
    setMessages((current) => [
      ...current,
      {
        id: `new-${current.length}`,
        user_id: "other",
        display_name: "Bob",
        parent_id: null,
        content: `New ${current.length}`,
        created_at: "2026-01-03 00:00:00.000",
      },
    ]);
  };
  return (
    <div
      ref={ref}
      data-testid="shell"
      data-app-shell
      className="fixed inset-x-0 top-0 flex h-dvh flex-col overflow-hidden"
    >
      <header className="flex shrink-0 items-center gap-2 border-b p-3">
        <span>Channel</span>
        <button type="button" data-testid="prepend-older" onClick={prependOlder}>
          Prepend older
        </button>
        <button type="button" data-testid="append-new" onClick={appendNew}>
          Append new
        </button>
      </header>
      <MessageList
        messages={messages}
        isLoading={false}
        isError={false}
        currentUserId="me"
        editingId={editingId}
        editingText={editingText}
        onEditingTextChange={setEditingText}
        onStartEdit={(id, content) => {
          setEditingId(id);
          setEditingText(content);
        }}
        onCancelEdit={() => setEditingId(null)}
        onSaveEdit={() => {
          setMessages(
            messages.map((message) =>
              message.id === editingId ? { ...message, content: editingText } : message,
            ),
          );
          setEditingId(null);
        }}
        onRequestDelete={noop}
        reactions={new Map()}
        kitchenCatalog={null}
        canReact={false}
        onReaction={noop}
        onPickReaction={noop}
        replyCounts={new Map()}
        onOpenThread={noop}
      />
      <Composer onSend={async () => true} />
    </div>
  );
}
localStorage.setItem("zoo-locale", "en");
createRoot(document.getElementById("app")!).render(
  <LocaleProvider>
    <Layout />
  </LocaleProvider>,
);
