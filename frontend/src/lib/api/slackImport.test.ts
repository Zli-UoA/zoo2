import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import {
  applyBulkAction,
  applyBulkArchive,
  defaultMappings,
  mappingFor,
  targetChoices,
  uploadSlackExport,
  splitSlackImports,
  slackImportsSnapshot,
  type PutWithProgress,
  type SlackChannelSummary,
  type SlackImport,
} from "./slackImport";

function channel(overrides: Partial<SlackChannelSummary>): SlackChannelSummary {
  return {
    slack_channel_id: "C1",
    name: "general",
    is_private: false,
    is_archived: false,
    members: 1,
    creator: "U1",
    message_count: 1,
    suggested_name: "slack-general",
    name_taken: false,
    existing_channel: null,
    ...overrides,
  };
}

describe("defaultMappings", () => {
  it("imports nothing until the admin picks channels", () => {
    expect(defaultMappings([channel({}), channel({ slack_channel_id: "C2" })])).toEqual([
      { slack_channel_id: "C1", action: "skip" },
      { slack_channel_id: "C2", action: "skip" },
    ]);
  });
});

describe("mappingFor", () => {
  const targets = [{ id: "channel-random" }];
  it("creates with the suggested name and keeps private channels private", () => {
    expect(mappingFor(channel({ is_private: true }), "create", targets)).toEqual({
      slack_channel_id: "C1",
      action: "create",
      name: "slack-general",
      is_private: true,
    });
  });
  it("keeps the previous target for a channel imported before", () => {
    const imported = channel({ existing_channel: { id: "channel-old", name: "old" } });
    expect(mappingFor(imported, "existing", targets)).toEqual({
      slack_channel_id: "C1",
      action: "existing",
      channel_id: "channel-old",
    });
    expect(mappingFor(channel({}), "existing", targets)).toMatchObject({
      channel_id: "channel-random",
    });
  });
});

describe("applyBulkAction", () => {
  const channels = [
    channel({ slack_channel_id: "C1", name: "general", suggested_name: "slack-general" }),
    channel({
      slack_channel_id: "C2",
      name: "random",
      existing_channel: { id: "z-random", name: "random" },
    }),
    channel({ slack_channel_id: "C3", name: "design", deleted_channel: "slack-design" }),
    channel({ slack_channel_id: "C4", name: "dev", is_private: true, suggested_name: "slack-dev" }),
  ];

  it("creates every channel that can be created and leaves fixed targets alone", () => {
    const next = applyBulkAction(channels, defaultMappings(channels), "create");
    expect(next).toEqual([
      { slack_channel_id: "C1", action: "create", name: "slack-general", is_private: false },
      { slack_channel_id: "C2", action: "skip" },
      { slack_channel_id: "C3", action: "skip" },
      { slack_channel_id: "C4", action: "create", name: "slack-dev", is_private: true },
    ]);
  });

  it("keeps the edited name and the archive choice of a channel that is already a new channel", () => {
    const edited = defaultMappings(channels).map((mapping) =>
      mapping.slack_channel_id === "C1"
        ? ({
            ...mapping,
            action: "create",
            name: "mine",
            is_private: false,
            archive: true,
          } as const)
        : mapping,
    );
    const next = applyBulkAction(channels, edited, "create");
    expect(next[0]).toEqual({
      slack_channel_id: "C1",
      action: "create",
      name: "mine",
      is_private: false,
      archive: true,
    });
  });

  it("skips everything at once", () => {
    const created = applyBulkAction(channels, defaultMappings(channels), "create");
    expect(applyBulkAction(channels, created, "skip")).toEqual(defaultMappings(channels));
  });
});

describe("applyBulkArchive", () => {
  it("sets the archive choice only on new channels", () => {
    const mappings = [
      { slack_channel_id: "C1", action: "create", name: "a", is_private: false },
      { slack_channel_id: "C2", action: "existing", channel_id: "x" },
      { slack_channel_id: "C3", action: "skip" },
    ] as const;
    const on = applyBulkArchive([...mappings], true);
    expect(on[0]).toMatchObject({ action: "create", archive: true });
    expect(on[1]).toEqual(mappings[1]);
    expect(on[2]).toEqual(mappings[2]);
    expect(applyBulkArchive(on, false)[0]).toMatchObject({ archive: false });
  });
});

describe("targetChoices", () => {
  const targets = [
    { id: "public", channel_name: "public", is_private: false },
    { id: "secret", channel_name: "secret", is_private: true },
  ];
  it("keeps private history out of public channels", () => {
    expect(targetChoices(channel({ is_private: true }), targets).map((t) => t.id)).toEqual([
      "secret",
    ]);
    expect(targetChoices(channel({}), targets).map((t) => t.id)).toEqual(["public", "secret"]);
  });
  it("offers the previous target even when it is not synced", () => {
    // 参加していない非公開チャンネルは Shape に載らないが、以前の取り込み先には入れ直せる
    const imported = channel({
      is_private: true,
      existing_channel: { id: "old", name: "old-secret", is_private: true },
    });
    expect(targetChoices(imported, [])).toEqual([
      { id: "old", channel_name: "old-secret", is_private: true },
    ]);
    expect(targetChoices(imported, targets).map((t) => t.id)).toEqual(["old", "secret"]);
  });
});

describe("splitSlackImports", () => {
  const job = (id: string, status: SlackImport["status"], source_id: string | null = null) =>
    ({ id, status, source_id }) as SlackImport;

  it("keeps saved exports and unfinished imports operable", () => {
    const imports = [
      job("run-b", "importing", "export"),
      job("run-a", "failed", "export"),
      job("export", "ready"),
      job("run-z", "completed", "export"),
      job("old", "cancelled"),
    ];
    const { cards, history } = splitSlackImports(imports);
    expect(cards.map((j) => j.id)).toEqual(["export", "run-b", "run-a"]);
    expect(history.map((j) => j.id)).toEqual(["run-z", "old"]);
  });

  it("shows the latest completed import when nothing is running", () => {
    const imports = [
      job("export", "ready"),
      job("run-b", "completed", "export"),
      job("run-a", "completed", "export"),
    ];
    const { cards, history } = splitSlackImports(imports);
    expect(cards.map((j) => j.id)).toEqual(["export", "run-b"]);
    expect(history.map((j) => j.id)).toEqual(["run-a"]);
  });
});

describe("uploadSlackExport", () => {
  it("creates the job, uploads to the presigned URL with progress, then asks for analysis", async () => {
    const request = vi.fn(async (path: string) =>
      path === "/api/admin/slack-imports"
        ? {
            import: { id: "import-1" },
            upload_url: "https://storage.example/upload",
            upload_headers: { "content-type": "application/zip" },
          }
        : { id: "import-1", status: "analyzing" },
    );
    const client = { request } as unknown as ApiClient;
    const put = vi.fn<PutWithProgress>(async (_url, _file, _headers, onProgress) => {
      onProgress(5, 10);
    });
    const progress = vi.fn();
    const file = { name: "export.zip", size: 10 } as File;

    await expect(uploadSlackExport(client, file, progress, put)).resolves.toEqual({
      id: "import-1",
      status: "analyzing",
    });
    expect(request).toHaveBeenNthCalledWith(1, "/api/admin/slack-imports", {
      method: "POST",
      body: { file_name: "export.zip", file_size: 10 },
    });
    expect(put).toHaveBeenCalledWith(
      "https://storage.example/upload",
      file,
      { "content-type": "application/zip" },
      expect.any(Function),
      undefined,
    );
    expect(progress).toHaveBeenCalledWith(0.5);
    expect(request).toHaveBeenNthCalledWith(2, "/api/admin/slack-imports/import-1/uploaded", {
      method: "POST",
    });
  });

  it("reports the created job before uploading so a cancel can abort the upload", async () => {
    const request = vi.fn(async () => ({
      import: { id: "import-1", status: "uploading" },
      upload_url: "https://storage.example/upload",
      upload_headers: {},
    }));
    const client = { request } as unknown as ApiClient;
    const controller = new AbortController();
    const order: string[] = [];
    const put = vi.fn<PutWithProgress>(async (_url, _file, _headers, _onProgress, signal) => {
      order.push("put");
      expect(signal).toBe(controller.signal);
      controller.abort();
      throw new Error("aborted");
    });
    const created = vi.fn(() => order.push("created"));
    const file = { name: "export.zip", size: 10 } as File;

    await expect(
      uploadSlackExport(client, file, vi.fn(), put, controller.signal, created),
    ).rejects.toThrow("aborted");
    expect(created).toHaveBeenCalledWith({ id: "import-1", status: "uploading" });
    expect(order).toEqual(["created", "put"]);
    // 中断したら解析を頼まない
    expect(request).toHaveBeenCalledTimes(1);
  });

  it("does not ask for analysis when the upload fails", async () => {
    const request = vi.fn(async () => ({
      import: { id: "import-1" },
      upload_url: "https://storage.example/upload",
      upload_headers: {},
    }));
    const client = { request } as unknown as ApiClient;
    const put = vi.fn<PutWithProgress>(async () => {
      throw new Error("network");
    });

    await expect(
      uploadSlackExport(client, { name: "a.zip", size: 1 } as File, vi.fn(), put),
    ).rejects.toThrow("network");
    expect(request).toHaveBeenCalledTimes(1);
  });
});

describe("slackImportsSnapshot", () => {
  it("changes when jobs or their statuses change, not when only the progress does", () => {
    const job = {
      id: "run-1",
      status: "fetching_files",
      processed_messages: 1,
      processed_files: 0,
    } as unknown as SlackImport;
    const base = slackImportsSnapshot([job]);
    expect(slackImportsSnapshot([{ ...job, processed_files: 5 } as SlackImport])).toBe(base);
    expect(slackImportsSnapshot([{ ...job, status: "completed" } as SlackImport])).not.toBe(base);
    expect(slackImportsSnapshot([job, { ...job, id: "run-2" } as SlackImport])).not.toBe(base);
  });
});
