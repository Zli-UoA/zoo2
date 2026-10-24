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
      content: `Message ${i}\n` + "Original line\n".repeat((i % 4) + 1),
      created_at: "2026-01-02 00:00:00.000",
    })),
  );
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  const noop = () => {};
  return (
    <div
      ref={ref}
      data-testid="shell"
      className="fixed inset-x-0 top-0 flex h-dvh flex-col overflow-hidden"
    >
      <header className="shrink-0 border-b p-3">Channel</header>
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
