import type { ApiClient } from "./client";
import type { ChannelResponse, CreateDmRequest } from "./types";

export function createOrGetDm(
  client: ApiClient,
  targetUserIds: string[],
  options: { encrypted?: boolean } = {},
) {
  const body: CreateDmRequest = { target_user_ids: targetUserIds };
  // 平文の DM では、今までどおり `encrypted` を送らない。
  if (options.encrypted) body.encrypted = true;
  return client.post<ChannelResponse>("/api/dms", body);
}
