import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import {
  defaultMappings,
  mappingFor,
  uploadSlackExport,
  splitSlackImports,
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
