// @vitest-environment happy-dom
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import type {
  PersistenceAdapter,
  PersistedStorageTarget,
} from "@tanstack/db-sqlite-persistence-core";
import { afterEach, expect, it, vi } from "vite-plus/test";

import { resolveConfig } from "../config";
import { getCollections } from "./collections";
import { CollectionsProvider } from "./collections-context";
import {
  useActiveChannels,
  useChannelMembers,
  useMessageFeed,
  useUsers,
} from "../../hooks/useRealtime";

afterEach(() => vi.unstubAllGlobals());

it.each([true, false])(
  "restores cached history before network sync (cached: %s)",
  async (cached) => {
    const message = {
      id: "cached-message",
      channel_id: "cached-channel",
      user_id: "dev-user",
      content: "Saved history",
      parent_id: null,
      created_at: "2026-11-08T00:00:00Z",
      updated_at: null,
      deleted_at: null,
    };
    const user = { id: "dev-user", user_name: "alice", display_name: "Alice" };
    const channel = { id: message.channel_id, channel_name: "Saved channel", deleted_at: null };
    const membership = { user_id: user.id, channel_id: channel.id, joined_at: message.created_at };
    const snapshots: Record<string, { key: string; value: Record<string, unknown> }[]> = {
      users: [{ key: user.id, value: user }],
      channels: [{ key: channel.id, value: channel }],
      channel_user: [{ key: `${user.id}:${channel.id}`, value: membership }],
      [`messages:${channel.id}`]: [{ key: message.id, value: message }],
    };
    const cachedRows = (target: PersistedStorageTarget) => {
      if (!cached) return [];
      return target.kind === "eager" ? (snapshots[target.collectionId] ?? []) : [];
    };
    const adapter: PersistenceAdapter = {
      loadSubset: async (target) => cachedRows(target),
      loadCollectionMetadata: async () => [],
      loadResumeSnapshot: async (target) => ({
        rows: cachedRows(target),
        collectionMetadata: [],
        latestTerm: 0,
        latestSeq: 0,
        latestRowVersion: 0,
        resetEpoch: 0,
      }),
      applyCommittedTx: async () => {},
      ensureIndex: async () => {},
    };
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(
        (_input, init) =>
          new Promise((_resolve, reject) => {
            init?.signal?.addEventListener("abort", () => reject(init.signal?.reason), {
              once: true,
            });
          }),
      ),
    );
    const bundle = getCollections(
      resolveConfig({ DEV: true, VITE_SHAPE_MODE: "proxy", VITE_DEV_USER_ID: "dev-user" }),
      { namespace: `cached-history-test:${cached}`, persistence: { adapter } },
    );
    const messages = bundle.messages(message.channel_id);
    const { result, unmount } = renderHook(
      () => ({
        feed: useMessageFeed(message.channel_id),
        channels: useActiveChannels(),
        users: useUsers(),
        members: useChannelMembers(message.channel_id),
      }),
      {
        wrapper: ({ children }: { children: ReactNode }) =>
          createElement(CollectionsProvider, { value: bundle, children }),
      },
    );
    try {
      await waitFor(
        () => {
          for (const query of Object.values(result.current)) {
            expect(query.isPersistedReady).toBe(true);
            expect(query.isLoading).toBe(!cached);
          }
          expect(result.current.channels.isInitialSyncLoading).toBe(true);
          expect(result.current.feed.data?.map((row) => row.content)).toEqual(
            cached ? ["Saved history"] : [],
          );
        },
        { timeout: 1_000 },
      );
    } finally {
      unmount();
      await Promise.all([
        messages.cleanup(),
        bundle.channels.cleanup(),
        bundle.users.cleanup(),
        bundle.channelUsers.cleanup(),
        bundle.reactions(message.channel_id).cleanup(),
      ]);
    }
  },
);
