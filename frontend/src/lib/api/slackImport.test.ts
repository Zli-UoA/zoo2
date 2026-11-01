import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "./client";
import {
  defaultMappings,
  uploadSlackExport,
  type PutWithProgress,
  type SlackChannelSummary,
} from "./slackImport";

function channel(overrides: Partial<SlackChannelSummary>): SlackChannelSummary {
  return {
    slack_channel_id: "C1",
    name: "general",
    is_private: false,
    is_archived: false,
    members: 1,
    message_count: 1,
    suggested_name: "slack-general",
    name_taken: false,
    existing_channel: null,
    ...overrides,
  };
}

describe("defaultMappings", () => {
  it("continues into a previously mapped channel and otherwise creates one", () => {
    expect(
      defaultMappings([
        channel({}),
        channel({
          slack_channel_id: "G1",
          is_private: true,
          suggested_name: "secret",
        }),
        channel({
          slack_channel_id: "C2",
          existing_channel: { id: "channel-random", name: "random" },
        }),
        channel({ slack_channel_id: "C3", deleted_channel: "old-random" }),
      ]),
    ).toEqual([
      { slack_channel_id: "C1", action: "create", name: "slack-general", is_private: false },
      { slack_channel_id: "G1", action: "create", name: "secret", is_private: true },
      { slack_channel_id: "C2", action: "existing", channel_id: "channel-random" },
      { slack_channel_id: "C3", action: "skip" },
    ]);
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
