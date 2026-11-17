import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";
import interactive from "../../../../../crates/zoo-components/fixtures/valid/message-interactive.json";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ComponentTree } from "./ComponentTree";
import { parseComponents } from "./parse";

const { api, collections } = vi.hoisted(() => ({
  api: { request: vi.fn(async () => ({})), get: vi.fn() },
  // ユーザーとチャンネルの collection がない（同期前・Storybook）状態
  collections: { config: { apiBaseUrl: "" } },
}));
vi.mock("../../../lib/electric/collections-context", () => ({
  useCollections: () => collections,
  useOptionalCollections: () => collections,
}));
vi.mock("../../../lib/api/client", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  createConfiguredApiClient: () => api,
}));

const doc = parseComponents(interactive.document)!;

describe("ComponentTree", () => {
  it("renders every interactive component without synced users or channels", async () => {
    render(
      <LocaleProvider>
        <ComponentTree document={doc} />
      </LocaleProvider>,
    );
    // アプリがない（Webhook と同じ）ので押せない
    expect(await screen.findByRole("button", { name: "Approve" })).toBeDisabled();
    // Label の外のピッカーにも名前がある（axe の label ルール）
    for (const name of ["日付", "時刻", "日時"])
      expect(screen.getByLabelText(name)).toBeInTheDocument();
  });

  it("falls back to the default name when a picker placeholder is blank", async () => {
    const blank = parseComponents({
      components: [
        {
          type: 1,
          components: [{ type: 1003, custom_id: "d", placeholder: "" }],
        },
        {
          type: 1,
          components: [{ type: 1004, custom_id: "t", placeholder: "   " }],
        },
      ],
    })!;
    render(
      <LocaleProvider>
        <ComponentTree document={blank} />
      </LocaleProvider>,
    );
    expect(await screen.findByLabelText("日付")).toBeInTheDocument();
    expect(screen.getByLabelText("時刻")).toBeInTheDocument();
  });

  it("names a string select without placeholder or label", async () => {
    const select = parseComponents({
      components: [
        {
          type: 1,
          components: [{ type: 3, custom_id: "s", options: [{ label: "A", value: "a" }] }],
        },
      ],
    })!;
    render(
      <LocaleProvider>
        <ComponentTree document={select} />
      </LocaleProvider>,
    );
    expect(await screen.findByRole("combobox")).toHaveAccessibleName();
  });

  it("asks for confirmation before sending and sends picker values only on submit", async () => {
    render(
      <LocaleProvider>
        <ComponentTree document={doc} appId="app" messageId="m1" channelId="c1" />
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(api.request).not.toHaveBeenCalled();
    await userEvent.click(await screen.findByRole("button", { name: "Yes" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    expect(JSON.stringify(api.request.mock.calls[0])).toContain('"approve":"yes"');
  });
  it("keeps an app-disabled field fixed inside a submit form", async () => {
    api.request.mockClear();
    const form = parseComponents({
      components: [
        {
          type: 18,
          label: "Fixed",
          component: { type: 23, custom_id: "fixed", disabled: true, default: false },
        },
      ],
    })!;
    render(
      <LocaleProvider>
        <ComponentTree document={form} appId="app" surfaceId="s1" submit />
      </LocaleProvider>,
    );
    const box = await screen.findByRole("checkbox");
    expect(box).toBeDisabled();
    await userEvent.click(box);
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    expect(JSON.stringify(api.request.mock.calls[0])).toContain('"fixed":false');
  });

  it("sends the labelled inputs of an app message with a submit button", async () => {
    api.request.mockClear();
    const message = parseComponents({
      components: [
        { type: 18, label: "Name", component: { type: 4, custom_id: "name", style: 1 } },
      ],
    })!;
    render(
      <LocaleProvider>
        <ComponentTree document={message} appId="app" messageId="m1" channelId="c1" />
      </LocaleProvider>,
    );
    await userEvent.type(await screen.findByRole("textbox", { name: "Name" }), "zoo");
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    const call = JSON.stringify(api.request.mock.calls[0]);
    expect(call).toContain('"kind":"submit"');
    expect(call).toContain('"name":"zoo"');
  });
  it("still sends selects outside the Labels of an app message when chosen", async () => {
    api.request.mockClear();
    const message = parseComponents({
      components: [
        {
          type: 1,
          components: [{ type: 21, custom_id: "vote", options: [{ label: "Yes", value: "y" }] }],
        },
        { type: 18, label: "Name", component: { type: 4, custom_id: "name", style: 1 } },
      ],
    })!;
    render(
      <LocaleProvider>
        <ComponentTree document={message} appId="app" messageId="m1" channelId="c1" />
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("radio", { name: "Yes" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    const action = JSON.stringify(api.request.mock.calls[0]);
    expect(action).toContain('"kind":"action"');
    expect(action).toContain('"action_id":"vote"');
    // Label の外の radio は送信の必須にならない
    await userEvent.type(screen.getByRole("textbox", { name: "Name" }), "zoo");
    api.request.mockClear();
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    expect(JSON.stringify(api.request.mock.calls[0])).toContain('"kind":"submit"');
  });
  it("keeps the old choice when the confirmation is declined", async () => {
    api.request.mockClear();
    const doc = parseComponents({
      components: [
        {
          type: 1,
          components: [
            {
              type: 21,
              custom_id: "plan",
              options: [
                { label: "Free", value: "free", default: true },
                { label: "Paid", value: "paid" },
              ],
              confirm: { title: "Change?", text: "Sure?", confirm: "Yes", deny: "No" },
            },
          ],
        },
      ],
    })!;
    render(
      <LocaleProvider>
        <ComponentTree document={doc} appId="app" messageId="m1" channelId="c1" />
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("radio", { name: "Paid" }));
    await userEvent.click(await screen.findByRole("button", { name: "No" }));
    expect(api.request).not.toHaveBeenCalled();
    expect(screen.getByRole("radio", { name: "Free" })).toBeChecked();
  });

  it("resets the inputs when the app updates the document", async () => {
    const make = (value: string) =>
      parseComponents({
        components: [
          { type: 18, label: "Name", component: { type: 4, custom_id: "name", style: 1, value } },
        ],
      })!;
    const view = render(
      <LocaleProvider>
        <ComponentTree document={make("first")} appId="app" surfaceId="s1" submit />
      </LocaleProvider>,
    );
    expect(await screen.findByRole("textbox", { name: "Name" })).toHaveValue("first");
    view.rerender(
      <LocaleProvider>
        <ComponentTree document={make("second")} appId="app" surfaceId="s1" submit />
      </LocaleProvider>,
    );
    expect(await screen.findByRole("textbox", { name: "Name" })).toHaveValue("second");
  });
  it("shows and sends a checkbox the app added in an update", async () => {
    api.request.mockClear();
    const first = { type: 18, label: "First", component: { type: 23, custom_id: "first" } };
    const second = {
      type: 18,
      label: "Second",
      component: { type: 23, custom_id: "second", default: true },
    };
    const view = render(
      <LocaleProvider>
        <ComponentTree
          document={parseComponents({ components: [first] })!}
          appId="app"
          surfaceId="s1"
          submit
        />
      </LocaleProvider>,
    );
    await screen.findByRole("checkbox", { name: "First" });
    view.rerender(
      <LocaleProvider>
        <ComponentTree
          document={parseComponents({ components: [first, second] })!}
          appId="app"
          surfaceId="s1"
          submit
        />
      </LocaleProvider>,
    );
    expect(await screen.findByRole("checkbox", { name: "Second" })).toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await waitFor(() => expect(api.request).toHaveBeenCalledTimes(1));
    const body = JSON.stringify(api.request.mock.calls[0]);
    expect(body).toContain('"first":false');
    expect(body).toContain('"second":true');
  });

  it("keeps typed input when the same document renders again", async () => {
    const doc = {
      components: [
        { type: 18, label: "Name", component: { type: 4, custom_id: "name", style: 1 } },
      ],
    };
    const view = render(
      <LocaleProvider>
        <ComponentTree document={parseComponents(doc)!} appId="app" surfaceId="s1" submit />
      </LocaleProvider>,
    );
    await userEvent.type(await screen.findByRole("textbox", { name: "Name" }), "zoo");
    // 内容が同じ文書（別のオブジェクト）で描画し直しても、入力中の値は消えない
    view.rerender(
      <LocaleProvider>
        <ComponentTree document={parseComponents(doc)!} appId="app" surfaceId="s1" submit />
      </LocaleProvider>,
    );
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue("zoo");
  });
  it("keeps the table page in range when the page size changes", async () => {
    const table = (page_size: number) =>
      parseComponents({
        components: [{ type: 1009, page_size, rows: [[{ text: "row-a" }], [{ text: "row-b" }]] }],
      })!;
    const view = render(
      <LocaleProvider>
        <ComponentTree document={table(1)} />
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "›" }));
    expect(await screen.findByText("row-b")).toBeInTheDocument();
    view.rerender(
      <LocaleProvider>
        <ComponentTree document={table(0)} />
      </LocaleProvider>,
    );
    expect(await screen.findByText("row-a")).toBeInTheDocument();
    expect(screen.getByText("row-b")).toBeInTheDocument();
  });
});
