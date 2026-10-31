import { useEffect, useRef, useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import ja from "../../lib/i18n/locales/ja.json";
import en from "../../lib/i18n/locales/en.json";
import { MessageRow } from "../chat/messages/MessageRow";

interface Props {
  compare: boolean;
  sample: "message" | "composer" | "emoji-picker" | "rich-text";
  content: string;
  disabled: boolean;
  grouped: boolean;
  dark: boolean;
  locale: "ja" | "en";
}

function flatten(value: object, prefix = ""): Record<string, string> {
  return Object.fromEntries(
    Object.entries(value).flatMap(([key, item]) => {
      const path = prefix ? `${prefix}.${key}` : key;
      return typeof item === "string" ? [[path, item]] : Object.entries(flatten(item, path));
    }),
  );
}
const labels = { ja: flatten(ja), en: flatten(en) };

function ComposeCatalog(props: Props) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [state, setState] = useState("loading");
  const [defaultMessage, setDefaultMessage] = useState("");
  const options = { ...props, labels: labels[props.locale] };
  const current = useRef(options);
  current.current = options;
  const send = () =>
    frame.current?.contentWindow?.postMessage(
      {
        type: "zoo-compose-catalog-options",
        options: current.current,
      },
      window.location.origin,
    );
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (
        event.origin !== window.location.origin ||
        event.source !== frame.current?.contentWindow ||
        event.data?.type !== "zoo-compose-catalog-status"
      )
        return;
      setState(event.data.state);
      if (typeof event.data.defaultMessage === "string")
        setDefaultMessage(event.data.defaultMessage);
      if (event.data.state === "waiting") send();
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, []);
  useEffect(send, [
    props.sample,
    props.content,
    props.disabled,
    props.grouped,
    props.dark,
    props.locale,
  ]);
  const noop = () => {};
  const native = (
    <iframe
      ref={frame}
      title="Compose component catalog"
      src="/compose-catalog.html"
      onLoad={send}
      data-compose-catalog-state={state}
      style={{ display: "block", border: 0, width: "100%", height: "100dvh", minHeight: 480 }}
    />
  );
  if (!props.compare) return native;
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 360px), 1fr))",
      }}
    >
      <section aria-label="React">
        <h2 className="px-5 py-2 text-sm font-semibold">React</h2>
        {defaultMessage && (
          <div className="py-6">
            <MessageRow
              item={{
                message: {
                  id: "catalog-message",
                  user_id: "alice",
                  user_name: "alice",
                  display_name: "Alice",
                  content: props.content || defaultMessage,
                  created_at: "2026-01-02T09:00:00Z",
                  parent_id: null,
                },
                isGroupStart: !props.grouped,
                isGroupEnd: true,
                showDateSeparator: false,
              }}
              isOwn={true}
              isEditing={false}
              editingText=""
              onEditingTextChange={noop}
              onStartEdit={noop}
              onCancelEdit={noop}
              onSaveEdit={noop}
              onRequestDelete={noop}
              reactions={[]}
              kitchenCatalog={null}
              canReact={true}
              onReaction={noop}
              onPickReaction={noop}
            />
          </div>
        )}
      </section>
      <section aria-label="Compose">
        <h2 className="px-5 py-2 text-sm font-semibold">Compose</h2>
        {native}
      </section>
    </div>
  );
}

const meta = {
  title: "Compose/Components",
  component: ComposeCatalog,
  // Canvas interactions are covered by Playwright against the built catalog.
  // Existing React story tests can run without a JDK or generated Wasm assets.
  tags: ["!test"],
  parameters: { layout: "fullscreen", a11y: { test: "off" } },
  args: {
    compare: false,
    sample: "message",
    content: "",
    disabled: false,
    grouped: false,
    dark: false,
    locale: "ja",
  },
  argTypes: {
    compare: { control: false, table: { disable: true } },
    sample: { control: "select", options: ["message", "composer", "emoji-picker", "rich-text"] },
    locale: { control: false, table: { disable: true } },
    dark: { control: false, table: { disable: true } },
  },
  render: (args, context) => (
    <ComposeCatalog
      {...args}
      dark={context.globals.theme === "dark"}
      locale={context.globals.locale === "en" ? "en" : "ja"}
    />
  ),
} satisfies Meta<typeof ComposeCatalog>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Message: Story = {};
export const GroupedMessage: Story = { args: { grouped: true } };
export const Composer: Story = { args: { sample: "composer" } };
export const DisabledComposer: Story = { args: { sample: "composer", disabled: true } };
export const EmojiPicker: Story = { args: { sample: "emoji-picker" } };
export const RichText: Story = { args: { sample: "rich-text" } };

export const ReactComparison: Story = {
  args: { compare: true },
  argTypes: { sample: { control: false } },
};
