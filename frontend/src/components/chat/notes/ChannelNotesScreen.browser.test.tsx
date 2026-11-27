import "../../../styles.css";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { page } from "vite-plus/test/browser/context";

import type {
  ChannelNote,
  ChannelNoteSummary,
  DeletedChannelNote,
  DeletedChannelNoteSummary,
} from "../../../lib/api/channelNotes";
import { DEFAULT_CONFIG } from "../../../lib/config";
import { mockServer, startMockServer } from "../../../test/browser-setup";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ChannelNotesScreen } from "./ChannelNotesScreen";

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useBlocker: () => {},
}));

vi.mock("../../../lib/electric/collections-context", async (importOriginal) => {
  // 毎 render で新しい config を返すと API の useMemo が変わり、一覧 effect が
  // 再起動して Maximum update depth になる。モジュールで 1 度だけ作る。
  const config = { ...DEFAULT_CONFIG, telemetryEnabled: false };
  return {
    ...(await importOriginal<typeof import("../../../lib/electric/collections-context")>()),
    useCollections: () => ({ config }),
  };
});

const CHANNEL = "11111111-1111-4111-8111-111111111111";
const TIMESTAMP = "2026-11-28T01:02:03.000";
const clipboard = { writeText: vi.fn(async () => {}) };

function summary(index: number, overrides: Partial<ChannelNoteSummary> = {}): ChannelNoteSummary {
  return {
    id: `note-${index}`,
    channel_id: CHANNEL,
    title: `Note ${index}`,
    position: index,
    version: 1,
    created_user_id: "alice",
    created_at: TIMESTAMP,
    updated_at: TIMESTAMP,
    order_token: "token-a",
    ...overrides,
  };
}

function noteBody(id: string, content: string, overrides: Partial<ChannelNote> = {}): ChannelNote {
  return {
    id,
    channel_id: CHANNEL,
    title: `Note ${id}`,
    content,
    position: 0,
    version: 1,
    created_user_id: "alice",
    created_at: TIMESTAMP,
    updated_at: TIMESTAMP,
    ...overrides,
  };
}

function deletedSummary(
  id: string,
  overrides: Partial<DeletedChannelNoteSummary> = {},
): DeletedChannelNoteSummary {
  return {
    ...summary(0, { id, title: `Deleted ${id}` }),
    deleted_at: "2026-11-27T12:00:00.000",
    ...overrides,
  };
}

function deletedBody(id: string, content: string): DeletedChannelNote {
  return {
    ...noteBody(id, content),
    deleted_at: "2026-11-27T12:00:00.000",
  };
}

function envelope<T>(data: T) {
  return HttpResponse.json({ data, txid: null });
}

function renderNotes(
  overrides: Partial<ComponentProps<typeof ChannelNotesScreen>> = {},
): ReturnType<typeof render> & { props: ComponentProps<typeof ChannelNotesScreen> } {
  const props: ComponentProps<typeof ChannelNotesScreen> = {
    channelId: CHANNEL,
    currentUserId: "alice",
    canRead: true,
    canWrite: true,
    isMember: true,
    isArchived: false,
    ...overrides,
  };
  const view = render(
    <LocaleProvider>
      <div style={{ display: "flex", height: 700, width: 1024 }}>
        <ChannelNotesScreen {...props} />
      </div>
    </LocaleProvider>,
  );
  return { ...view, props };
}

function updateProps(
  view: ReturnType<typeof renderNotes>,
  overrides: Partial<ComponentProps<typeof ChannelNotesScreen>>,
) {
  const props = { ...view.props, ...overrides };
  view.rerender(
    <LocaleProvider>
      <div style={{ display: "flex", height: 700, width: 1024 }}>
        <ChannelNotesScreen {...props} />
      </div>
    </LocaleProvider>,
  );
  view.props = props;
}

beforeEach(async () => {
  await startMockServer();
  await page.viewport(1024, 800);
  clipboard.writeText.mockClear();
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: clipboard,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

it("shows the note list and fetches only the selected note body", async () => {
  const listRequests: string[] = [];
  const detailRequests: string[] = [];
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", ({ request }) => {
      listRequests.push(new URL(request.url).search);
      return envelope([summary(0), summary(1)]);
    }),
    http.get("*/api/channels/:channelId/notes/:noteId", ({ params }) => {
      detailRequests.push(String(params.noteId));
      return envelope(noteBody(String(params.noteId), `Body ${String(params.noteId)}`));
    }),
  );

  renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "Note 0" }));

  expect(await screen.findByText("Body note-0")).toBeVisible();
  expect(listRequests).toEqual(["?limit=50"]);
  expect(detailRequests).toEqual(["note-0"]);
  expect(screen.queryByText("Body note-1")).toBeNull();
});

it("switches from the list to the body and back on a narrow screen", async () => {
  await page.viewport(390, 844);
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0)])),
    http.get("*/api/channels/:channelId/notes/:noteId", () =>
      envelope(noteBody("note-0", "Mobile body")),
    ),
  );

  renderNotes();
  expect(screen.queryByRole("button", { name: "ノート一覧に戻る" })).toBeNull();
  await userEvent.click(await screen.findByRole("button", { name: "Note 0" }));
  expect(await screen.findByText("Mobile body")).toBeVisible();

  await userEvent.click(screen.getByRole("button", { name: "ノート一覧に戻る" }));
  expect(await screen.findByRole("button", { name: "Note 0" })).toBeVisible();
  expect(screen.queryByText("Mobile body")).toBeNull();
});

it("keeps the server order and uses the last item as the page cursor", async () => {
  const cursors: string[] = [];
  const firstPage = Array.from({ length: 50 }, (_, index) =>
    summary(index, { order_token: "token-a" }),
  );
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", ({ request }) => {
      const url = new URL(request.url);
      const after = url.searchParams.get("after");
      cursors.push(after ?? "");
      return after
        ? envelope([
            summary(50, { order_token: "token-b" }),
            summary(51, { order_token: "token-b" }),
          ])
        : envelope(firstPage);
    }),
  );

  renderNotes();
  await screen.findByRole("button", { name: "Note 0" });
  await userEvent.click(screen.getByRole("button", { name: "さらに読み込む" }));

  expect(await screen.findByRole("button", { name: "Note 51" })).toBeVisible();
  expect(cursors).toEqual(["", "token-a 49 note-49"]);
  const titles = screen.getAllByRole("listitem").map((item) => item.textContent);
  expect(titles[0]).toContain("Note 0");
  expect(titles.at(-1)).toContain("Note 51");
});

it("reloads from the first page when the page cursor becomes stale", async () => {
  let firstPageRequests = 0;
  let secondPageRequests = 0;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", ({ request }) => {
      const url = new URL(request.url);
      if (!url.searchParams.get("after")) {
        firstPageRequests += 1;
        return envelope(
          firstPageRequests === 1
            ? Array.from({ length: 50 }, (_, index) => summary(index))
            : [summary(0, { title: "Reloaded note" })],
        );
      }
      secondPageRequests += 1;
      return new HttpResponse(null, { status: 409 });
    }),
  );

  renderNotes();
  await screen.findByRole("button", { name: "Note 0" });
  await userEvent.click(screen.getByRole("button", { name: "さらに読み込む" }));

  expect(
    await screen.findByText("一覧が更新されたため、先頭から読み込み直しました。"),
  ).toBeVisible();
  expect(await screen.findByRole("button", { name: "Reloaded note" })).toBeVisible();
  expect(secondPageRequests).toBe(1);
  expect(firstPageRequests).toBe(2);
});

it("saves a new note once with the raw title and keeps a FEFF title visible", async () => {
  const posts: Array<{ title: string; content: string }> = [];
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([])),
    http.post("*/api/channels/:channelId/notes", async ({ request }) => {
      const body = (await request.json()) as { title: string; content: string };
      posts.push(body);
      await pending;
      return envelope(noteBody("created", body.content, { title: "\uFEFF" }));
    }),
  );

  renderNotes();
  const newNote = await screen.findByRole("button", { name: "新規ノート" });
  await waitFor(() => expect(newNote).not.toBeDisabled());
  await userEvent.click(newNote);
  await userEvent.type(screen.getByRole("textbox", { name: "タイトル" }), "\uFEFF");
  await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Created body");
  const save = screen.getByRole("button", { name: "保存" });
  fireEvent.click(save);
  fireEvent.click(save);

  try {
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ title: "\uFEFF", content: "Created body" });
    expect((save as HTMLButtonElement).disabled).toBe(true);
  } finally {
    release();
  }
  expect(await screen.findByText("Created body")).toBeVisible();
  const contentRegion = screen.getByRole("region", { name: "ノート本文" });
  expect(within(contentRegion).getByRole("heading").textContent).toBe("\uFEFF");
});

it("keeps the draft when create reaches the 500-note limit", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([])),
    http.post("*/api/channels/:channelId/notes", () => new HttpResponse(null, { status: 409 })),
  );
  renderNotes();
  const newNote = await screen.findByRole("button", { name: "新規ノート" });
  await waitFor(() => expect(newNote).not.toBeDisabled());
  await userEvent.click(newNote);
  await userEvent.type(screen.getByRole("textbox", { name: "タイトル" }), "Runbook");
  await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Draft body");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));

  expect(
    await screen.findByText("有効なノートが上限（500件）に達しているため作成できません。"),
  ).toBeVisible();
  expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Draft body");
});

it("keeps a draft after an unknown create result and only resends on request", async () => {
  let posts = 0;
  let lists = 0;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => {
      lists += 1;
      return envelope([]);
    }),
    http.post("*/api/channels/:channelId/notes", async ({ request }) => {
      posts += 1;
      const body = (await request.json()) as { title: string; content: string };
      if (posts === 1) return new HttpResponse(null, { status: 503 });
      return envelope(noteBody("created", body.content));
    }),
  );

  renderNotes();
  const newNote = await screen.findByRole("button", { name: "新規ノート" });
  await waitFor(() => expect(newNote).not.toBeDisabled());
  await userEvent.click(newNote);
  await userEvent.type(screen.getByRole("textbox", { name: "タイトル" }), "Runbook");
  await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Draft body");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));

  expect(
    await screen.findByText(
      "作成結果を確認できませんでした。自動では再送しません。一覧を再取得しました。",
    ),
  ).toBeVisible();
  expect(screen.getByRole("textbox", { name: "タイトル" })).toHaveValue("Runbook");
  expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Draft body");
  await waitFor(() => expect(lists).toBeGreaterThanOrEqual(2));
  expect(posts).toBe(1);

  await userEvent.click(screen.getByRole("button", { name: "重複を承知で再送" }));
  await waitFor(() => expect(posts).toBe(2));
  expect(await screen.findByText("Draft body")).toBeVisible();
});

it("drops the deleted list and a delayed deleted body when membership is lost", async () => {
  await page.viewport(390, 844);
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  let deletedBodyRequests = 0;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0)])),
    http.get("*/api/channels/:channelId/notes/deleted", () => envelope([deletedSummary("del-1")])),
    http.get("*/api/channels/:channelId/notes/deleted/:noteId", async () => {
      deletedBodyRequests += 1;
      await pending;
      return envelope(deletedBody("del-1", "Deleted body"));
    }),
  );

  const view = renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "削除済み" }));
  await userEvent.click(await screen.findByRole("button", { name: /Deleted del-1/ }));
  await waitFor(() => expect(deletedBodyRequests).toBe(1));

  updateProps(view, { isMember: false, canWrite: false });
  expect(await screen.findByRole("button", { name: "Note 0" })).toBeVisible();
  expect(screen.queryByRole("button", { name: /Deleted del-1/ })).toBeNull();
  expect(screen.queryByRole("button", { name: "削除済み" })).toBeNull();

  release();
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(screen.queryByText("Deleted body")).toBeNull();
});

it("removes deleted content in the same commit when membership is lost", async () => {
  await page.viewport(390, 844);
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0)])),
    http.get("*/api/channels/:channelId/notes/deleted", () => envelope([deletedSummary("del-1")])),
    http.get("*/api/channels/:channelId/notes/deleted/:noteId", () =>
      envelope(deletedBody("del-1", "Deleted body")),
    ),
  );

  const view = renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "削除済み" }));
  await userEvent.click(await screen.findByRole("button", { name: /Deleted del-1/ }));
  expect(await screen.findByText("Deleted body")).toBeVisible();

  updateProps(view, { isMember: false, canWrite: false });
  // passive effect の後片付けを待たず、同じ commit の DOM から消えている。
  expect(screen.queryByText("Deleted body")).toBeNull();
  expect(screen.queryByRole("button", { name: /Deleted del-1/ })).toBeNull();
  expect(screen.getByRole("button", { name: "Note 0" })).toBeVisible();
});

it("keeps a conflicted draft and overwrites only after explicit confirmation", async () => {
  const patches: Array<{ title: string; content: string; version: number }> = [];
  let conflicted = false;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0)])),
    http.get("*/api/channels/:channelId/notes/:noteId", () =>
      envelope(
        noteBody("note-0", conflicted ? "Latest body" : "Base body", {
          title: "Note 0",
          version: conflicted ? 2 : 1,
        }),
      ),
    ),
    http.patch("*/api/channels/:channelId/notes/:noteId", async ({ request }) => {
      const body = (await request.json()) as {
        title: string;
        content: string;
        version: number;
      };
      patches.push(body);
      if (patches.length === 1) {
        conflicted = true;
        return new HttpResponse(null, { status: 409 });
      }
      return envelope(noteBody("note-0", body.content, { version: 3 }));
    }),
  );

  renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "Note 0" }));
  await userEvent.click(await screen.findByRole("button", { name: "編集" }));
  const textarea = screen.getByRole("textbox", { name: "本文" }) as HTMLTextAreaElement;
  await userEvent.clear(textarea);
  await userEvent.type(textarea, "Draft body");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));

  expect(
    await screen.findByText("他の参加者が先に更新しました。最新の内容と下書きを確認してください。"),
  ).toBeVisible();
  expect(await screen.findByText("Latest body")).toBeVisible();
  expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Draft body");
  expect(patches).toHaveLength(1);

  await userEvent.click(screen.getByRole("button", { name: "この下書きで上書き保存" }));
  expect(await screen.findByText("Draft body")).toBeVisible();
  expect(patches).toHaveLength(2);
  expect(patches[1]).toEqual({ title: "Note 0", content: "Draft body", version: 2 });
});

it("offers saving as a new note when an edit 404 is confirmed as deleted", async () => {
  const posts: Array<{ title: string; content: string }> = [];
  const patches: Array<{ title: string; content: string; version: number }> = [];
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0)])),
    http.get("*/api/channels/:channelId/notes/:noteId", () =>
      envelope(noteBody("note-0", "Base body", { title: "Note 0" })),
    ),
    http.patch("*/api/channels/:channelId/notes/:noteId", async ({ request }) => {
      patches.push((await request.json()) as { title: string; content: string; version: number });
      return new HttpResponse(null, { status: 404 });
    }),
    http.get("*/api/channels/:channelId/notes/deleted/:noteId", () =>
      envelope(deletedBody("note-0", "Base body")),
    ),
    http.post("*/api/channels/:channelId/notes", async ({ request }) => {
      posts.push((await request.json()) as { title: string; content: string });
      return envelope(noteBody("created", "Draft body"));
    }),
  );

  renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "Note 0" }));
  await userEvent.click(await screen.findByRole("button", { name: "編集" }));
  const textarea = screen.getByRole("textbox", { name: "本文" }) as HTMLTextAreaElement;
  await userEvent.clear(textarea);
  await userEvent.type(textarea, "Draft body");
  await userEvent.click(screen.getByRole("button", { name: "保存" }));

  expect(
    await screen.findByText("編集中のノートは削除されました。新規ノートとして保存できます。"),
  ).toBeVisible();
  expect(patches).toEqual([{ title: "Note 0", content: "Draft body", version: 1 }]);

  await userEvent.click(screen.getByRole("button", { name: "新規ノートとして保存" }));
  await waitFor(() => expect(posts).toHaveLength(1));
  expect(posts[0]).toEqual({ title: "Note 0", content: "Draft body" });
  expect(await screen.findByText("Draft body")).toBeVisible();
});

it("keeps the editor data locked while a save is in flight", async () => {
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  let patchBody: { title: string; content: string; version: number } | null = null;
  let patchStarted = false;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0), summary(1)])),
    http.get("*/api/channels/:channelId/notes/:noteId", ({ params }) =>
      envelope(noteBody(String(params.noteId), "Base body", { title: "Note 0" })),
    ),
    http.patch("*/api/channels/:channelId/notes/:noteId", async ({ request }) => {
      const body = (await request.json()) as {
        title: string;
        content: string;
        version: number;
      };
      patchBody = body;
      patchStarted = true;
      await pending;
      return envelope(noteBody("note-0", body.content, { version: 2 }));
    }),
  );

  renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "Note 0" }));
  await userEvent.click(await screen.findByRole("button", { name: "編集" }));
  const textarea = screen.getByRole("textbox", { name: "本文" }) as HTMLTextAreaElement;
  await userEvent.clear(textarea);
  await userEvent.type(textarea, "Draft body");
  try {
    await userEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(patchStarted).toBe(true));
    expect(textarea.readOnly).toBe(true);
    expect(screen.getByRole("button", { name: "キャンセル" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Note 1" })).toBeDisabled();
    await userEvent.type(textarea, " ignored");
    expect(textarea).toHaveValue("Draft body");
    expect(patchBody).toEqual({ title: "Note 0", content: "Draft body", version: 1 });
  } finally {
    release();
  }
  expect(await screen.findByText("Draft body")).toBeVisible();
});

it("keeps a copyable draft when only write access is lost and clears it when read access is lost", async () => {
  mockServer.use(http.get("*/api/channels/:channelId/notes", () => envelope([])));
  const view = renderNotes();
  const newNote = await screen.findByRole("button", { name: "新規ノート" });
  await waitFor(() => expect(newNote).not.toBeDisabled());
  await userEvent.click(newNote);
  await userEvent.type(screen.getByRole("textbox", { name: "タイトル" }), "Runbook");
  await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Draft body");

  updateProps(view, { isMember: false, canWrite: false });
  expect(
    await screen.findByText("書き込み権限がなくなりました。下書きはこの画面に残っています。"),
  ).toBeVisible();
  expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Draft body");
  expect(screen.queryByRole("button", { name: "保存" })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "下書きをコピー" }));
  expect(clipboard.writeText).toHaveBeenCalledWith("Runbook\n\nDraft body");

  updateProps(view, { canRead: false });
  expect(screen.getByRole("alert")).toHaveTextContent(
    "このチャンネルのノートを閲覧する権限がなくなりました。",
  );
  expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull();
});

it("enables refresh from the current list without opening deleted notes", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([])),
    http.get("*/api/channels/:channelId/notes/deleted", () => envelope([])),
  );

  renderNotes();
  await screen.findByText("ノートはありません。");
  expect(screen.getByRole("button", { name: "ノートを再読み込み" })).not.toBeDisabled();
});

it.each(["active", "deleted"] as const)(
  "keeps the %s list visible and offers retry when refreshing fails",
  async (kind) => {
    let requests = 0;
    mockServer.use(
      http.get("*/api/channels/:channelId/notes", () => {
        if (kind !== "active") return envelope([]);
        requests += 1;
        if (requests === 2) return new HttpResponse(null, { status: 503 });
        return envelope(
          Array.from({ length: 50 }, (_, index) =>
            summary(index, { title: requests === 1 ? `Stale ${index}` : `Fresh ${index}` }),
          ),
        );
      }),
      http.get("*/api/channels/:channelId/notes/deleted", () => {
        requests += 1;
        if (requests === 2) return new HttpResponse(null, { status: 503 });
        return envelope(
          Array.from({ length: 50 }, (_, index) =>
            deletedSummary(`del-${index}`, {
              title: requests === 1 ? `Stale ${index}` : `Fresh ${index}`,
            }),
          ),
        );
      }),
    );

    renderNotes();
    if (kind === "deleted") {
      await userEvent.click(screen.getByRole("button", { name: "削除済み" }));
    }
    expect(await screen.findByRole("button", { name: /Stale 0/ })).toBeVisible();
    expect(screen.getByRole("button", { name: "さらに読み込む" })).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "ノートを再読み込み" }));

    const alert = await screen.findByRole("alert");
    expect(
      within(alert).getByText(
        kind === "active"
          ? "ノートを読み込めませんでした。"
          : "削除済みノートを読み込めませんでした。",
      ),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: /Stale 0/ })).toBeVisible();
    expect(screen.queryByRole("button", { name: "さらに読み込む" })).toBeNull();
    await userEvent.click(within(alert).getByRole("button", { name: "再試行" }));
    expect(await screen.findByRole("button", { name: /Fresh 0/ })).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("button", { name: "さらに読み込む" })).toBeVisible();
  },
);

it("shows the UTC restore deadline, reads a deleted body, and opens the restored note", async () => {
  let restoreRequests = 0;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0)])),
    http.get("*/api/channels/:channelId/notes/deleted", () => envelope([deletedSummary("del-1")])),
    http.get("*/api/channels/:channelId/notes/deleted/:noteId", () =>
      envelope(deletedBody("del-1", "Deleted body")),
    ),
    http.post("*/api/channels/:channelId/notes/:noteId/restore", () => {
      restoreRequests += 1;
      return envelope(noteBody("del-1", "Restored body", { title: "Deleted del-1" }));
    }),
  );

  renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "削除済み" }));
  expect(await screen.findByText("復元期限: 2026/11/28 12:00 UTC")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: /Deleted del-1/ }));
  expect(await screen.findByText("Deleted body")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "復元" }));

  expect(await screen.findByText("Restored body")).toBeVisible();
  expect(restoreRequests).toBe(1);
  expect(screen.queryByRole("button", { name: /Deleted del-1/ })).toBeNull();
});

it("refreshes both lists and avoids claiming a reason when restore returns 404", async () => {
  const lists = { active: 0, deleted: 0 };
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => {
      lists.active += 1;
      return envelope([summary(0)]);
    }),
    http.get("*/api/channels/:channelId/notes/deleted", () => {
      lists.deleted += 1;
      return envelope(lists.deleted === 1 ? [deletedSummary("del-1")] : []);
    }),
    http.post(
      "*/api/channels/:channelId/notes/:noteId/restore",
      () => new HttpResponse(null, { status: 404 }),
    ),
  );

  renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "削除済み" }));
  await userEvent.click(await screen.findByRole("button", { name: "復元" }));

  expect(
    await screen.findByText(
      "復元できませんでした。期限切れか、既に復元・削除された可能性があります。",
    ),
  ).toBeVisible();
  await waitFor(() => expect(lists.active).toBeGreaterThanOrEqual(2));
  await waitFor(() => expect(lists.deleted).toBeGreaterThanOrEqual(2));
  expect(screen.queryByRole("button", { name: /Deleted del-1/ })).toBeNull();
});

it.each(["success", "404"] as const)(
  "keeps a newer deleted selection while a restore finishes with %s",
  async (outcome) => {
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    const listRequests = { active: 0, deleted: 0 };
    mockServer.use(
      http.get("*/api/channels/:channelId/notes", () => {
        listRequests.active += 1;
        return envelope([summary(0)]);
      }),
      http.get("*/api/channels/:channelId/notes/deleted", () => {
        listRequests.deleted += 1;
        return envelope([deletedSummary("del-1"), deletedSummary("del-2")]);
      }),
      http.get("*/api/channels/:channelId/notes/deleted/:noteId", ({ params }) =>
        envelope(
          deletedBody(
            String(params.noteId),
            params.noteId === "del-2" ? "Second body" : "First body",
          ),
        ),
      ),
      http.post("*/api/channels/:channelId/notes/:noteId/restore", async () => {
        await pending;
        return outcome === "success"
          ? envelope(noteBody("del-1", "Restored body"))
          : new HttpResponse(null, { status: 404 });
      }),
    );

    renderNotes();
    try {
      await userEvent.click(await screen.findByRole("button", { name: "削除済み" }));
      await userEvent.click(await screen.findByRole("button", { name: /Deleted del-1/ }));
      const firstRow = screen
        .getByRole("button", { name: /Deleted del-1/ })
        .closest("li") as HTMLElement;
      await userEvent.click(within(firstRow).getByRole("button", { name: "復元" }));

      await userEvent.click(screen.getByRole("button", { name: /Deleted del-2/ }));
      expect(await screen.findByText("Second body")).toBeVisible();

      release();
      // 復元応答後は両一覧の再取得が完了するまで待ち、その後に選択が
      // 切り替わっていないことを見る。
      await waitFor(() => expect(listRequests.active).toBeGreaterThanOrEqual(2));
      await waitFor(() => expect(listRequests.deleted).toBeGreaterThanOrEqual(2));
      expect(screen.getByText("Second body")).toBeVisible();
      expect(screen.queryByText("Restored body")).toBeNull();
    } finally {
      release();
    }
  },
);

it("handles the restore capacity limit", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope([summary(0)])),
    http.get("*/api/channels/:channelId/notes/deleted", () => envelope([deletedSummary("del-1")])),
    http.post(
      "*/api/channels/:channelId/notes/:noteId/restore",
      () => new HttpResponse(null, { status: 409 }),
    ),
  );

  renderNotes();
  await userEvent.click(await screen.findByRole("button", { name: "削除済み" }));
  await userEvent.click(await screen.findByRole("button", { name: "復元" }));

  expect(
    await screen.findByText("有効なノートが上限（500件）に達しているため復元できません。"),
  ).toBeVisible();
});

it("collects every page for reorder and saves the expected and new ids", async () => {
  const firstPage = Array.from({ length: 50 }, (_, index) => summary(index));
  const secondPage = [summary(50), summary(51)];
  let pageOneRequests = 0;
  let secondPageRequests = 0;
  const reorderBodies: Array<{ expected_note_ids: string[]; note_ids: string[] }> = [];
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", ({ request }) => {
      const url = new URL(request.url);
      if (url.searchParams.get("after")) {
        secondPageRequests += 1;
        // 並べ替え準備のページ取得だけ 1 回 409 にし、先頭からの再取得を確認する。
        if (secondPageRequests === 1) return new HttpResponse(null, { status: 409 });
        return envelope(secondPage);
      }
      pageOneRequests += 1;
      return envelope(firstPage);
    }),
    http.patch("*/api/channels/:channelId/notes/order", async ({ request }) => {
      reorderBodies.push(
        (await request.json()) as { expected_note_ids: string[]; note_ids: string[] },
      );
      return envelope({ updated: true });
    }),
  );

  renderNotes();
  const reorderButton = await screen.findByRole("button", { name: "並べ替え" });
  await waitFor(() => expect(reorderButton).not.toBeDisabled());
  await userEvent.click(reorderButton);
  expect(await screen.findByText("1. Note 0")).toBeVisible();
  expect(await screen.findByText("52. Note 51")).toBeVisible();
  expect(pageOneRequests).toBeGreaterThanOrEqual(2);
  expect(secondPageRequests).toBe(2);

  await userEvent.click(screen.getByRole("button", { name: "下へ移動: Note 0" }));
  expect(await screen.findByText("1. Note 1")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "順序を保存" }));

  expect(await screen.findByText("並び順を保存しました。")).toBeVisible();
  expect(reorderBodies).toHaveLength(1);
  expect(reorderBodies[0]!.expected_note_ids).toHaveLength(52);
  expect(reorderBodies[0]!.expected_note_ids.slice(0, 3)).toEqual(["note-0", "note-1", "note-2"]);
  expect(reorderBodies[0]!.note_ids.slice(0, 3)).toEqual(["note-1", "note-0", "note-2"]);
});

it("reorders by dragging on desktop and saves the dropped order", async () => {
  const items = [summary(0), summary(1), summary(2)];
  const bodies: Array<{ expected_note_ids: string[]; note_ids: string[] }> = [];
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope(items)),
    http.patch("*/api/channels/:channelId/notes/order", async ({ request }) => {
      bodies.push((await request.json()) as { expected_note_ids: string[]; note_ids: string[] });
      return envelope({ updated: true });
    }),
  );
  renderNotes();
  const reorderButton = await screen.findByRole("button", { name: "並べ替え" });
  await waitFor(() => expect(reorderButton).not.toBeDisabled());
  await userEvent.click(reorderButton);
  await screen.findByText("1. Note 0");

  const rows = screen.getAllByRole("listitem");
  const dataTransfer = new DataTransfer();
  fireEvent.dragStart(rows[0]!, { dataTransfer });
  fireEvent.dragOver(rows[2]!, { dataTransfer });
  fireEvent.drop(rows[2]!, { dataTransfer });
  expect(await screen.findByText("3. Note 0")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "順序を保存" }));

  expect(await screen.findByText("並び順を保存しました。")).toBeVisible();
  expect(bodies).toHaveLength(1);
  expect(bodies[0]!.note_ids).toEqual(["note-1", "note-2", "note-0"]);
});

it("disables editing actions while the reorder snapshot is loading", async () => {
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  let listRequests = 0;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", async () => {
      listRequests += 1;
      // 1 回目は初期一覧。2 回目（並べ替えの全ページ取得）だけ遅らせる。
      if (listRequests === 2) await pending;
      return envelope([summary(0), summary(1)]);
    }),
  );

  renderNotes();
  const reorderButton = await screen.findByRole("button", { name: "並べ替え" });
  await waitFor(() => expect(reorderButton).not.toBeDisabled());
  try {
    await userEvent.click(reorderButton);
    await waitFor(() => expect(listRequests).toBe(2));
    expect(screen.getByRole("button", { name: "新規ノート" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Note 0" })).toBeDisabled();
  } finally {
    release();
  }
  expect(await screen.findByText("1. Note 0")).toBeVisible();
  expect(screen.queryByRole("textbox", { name: "タイトル" })).toBeNull();
});

it("reloads the latest order after a reorder 409 without reapplying the old move", async () => {
  const original = [summary(0), summary(1), summary(2)];
  const latest = [summary(2), summary(1), summary(0)];
  let reorderRequests = 0;
  let orderChanged = false;
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => envelope(orderChanged ? latest : original)),
    http.patch("*/api/channels/:channelId/notes/order", () => {
      reorderRequests += 1;
      // 409 を返した時点で次の取得が新しい順序になる。alert 後に handler を
      // 差し替えると、再取得のタイミングがテストの待ちに依存する。
      orderChanged = true;
      return new HttpResponse(null, { status: 409 });
    }),
  );

  renderNotes();
  const reorderButton = await screen.findByRole("button", { name: "並べ替え" });
  await waitFor(() => expect(reorderButton).not.toBeDisabled());
  await userEvent.click(reorderButton);
  await screen.findByText("1. Note 0");
  await userEvent.click(screen.getByRole("button", { name: "下へ移動: Note 0" }));
  await userEvent.click(screen.getByRole("button", { name: "順序を保存" }));

  expect(
    await screen.findByText(
      "他の参加者が並び順を変更しました。最新の順序を表示しました。もう一度並べ替えて保存してください。",
    ),
  ).toBeVisible();
  expect(reorderRequests).toBe(1);
  await waitFor(() => expect(screen.getByText("1. Note 2")).toBeVisible());
  expect(screen.getByText("3. Note 0")).toBeVisible();
});
