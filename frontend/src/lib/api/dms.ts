import type { ApiClient } from "./client";
import type { ChannelResponse, CreateDmRequest } from "./types";

export function createOrGetDm(client: ApiClient, targetUserIds: string[]) {
  const body: CreateDmRequest = { target_user_ids: targetUserIds };
  return client.post<ChannelResponse>("/api/dms", body);
}
