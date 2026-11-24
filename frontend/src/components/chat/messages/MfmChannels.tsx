import { Link } from "@tanstack/react-router";
import { createContext, useContext, useMemo, type ReactNode } from "react";
import { noteEncryptedChannel } from "../../../lib/e2ee/encryptionGuard";
import type { ChannelRow } from "../../../lib/electric/types";

const MfmChannelsContext = createContext<ReadonlyMap<string, string>>(new Map());

/** メッセージ・チャンネルのリンクを短く描くために、リンク先の表示へ必要なチャンネルの情報。 */
export interface LinkedChannel {
  id: string;
  name: string;
  description: string | null;
  /** 非公開チャンネルと DM。 */
  isPrivate: boolean;
  isDm: boolean;
  /** 暗号化チャンネル。本文は埋め込みに出さない。 */
  encrypted: boolean;
}

/**
 * 自分が見られるチャンネルの一覧。サーバーの `channels` Shape が、公開チャンネルと自分が参加している
 * 非公開チャンネル・DM だけを配るので、ここにないチャンネルは「存在しない」ものとして扱う。
 * 一覧がまだ同期されていない間（`ready` が false）は、ないことを確定しない。
 */
export interface ChannelDirectory {
  byId: ReadonlyMap<string, LinkedChannel>;
  ready: boolean;
}

const ChannelDirectoryContext = createContext<ChannelDirectory | null>(null);

type ProvidedChannel = Pick<ChannelRow, "id" | "channel_name" | "deleted_at"> &
  Partial<Pick<ChannelRow, "description" | "is_private" | "encrypted">> & {
    is_dm?: boolean;
  };

/** Share the shell's live channel list with messages and thread replies. */
export function MfmChannelsProvider({
  channels,
  ready = true,
  children,
}: {
  channels: readonly ProvidedChannel[];
  /** 一覧の初回同期が終わったか。省くと終わっているものとして扱う。 */
  ready?: boolean;
  children: ReactNode;
}) {
  const byName = useMemo(
    () =>
      new Map(
        channels
          .filter((channel) => channel.deleted_at === null && !channel.is_dm)
          .map((channel) => [channel.channel_name, channel.id]),
      ),
    [channels],
  );
  const directory = useMemo<ChannelDirectory>(() => {
    const visible = channels.filter((channel) => channel.deleted_at === null);
    // 一度 `encrypted = true` を見たチャンネルは覚える。あとで `encrypted = false` が届いても（格下げ）、
    // 埋め込みに本文を出さないため（lib/e2ee/encryptionGuard.ts、docs/e2ee.md の「暗号化の格下げを防ぐ」）。
    for (const channel of visible) if (channel.encrypted) noteEncryptedChannel(channel.id);
    return {
      byId: new Map(
        visible.map((channel) => [
          channel.id,
          {
            id: channel.id,
            name: channel.channel_name,
            description: channel.description ?? null,
            isPrivate: channel.is_private ?? false,
            isDm: channel.is_dm ?? false,
            encrypted: channel.encrypted ?? false,
          },
        ]),
      ),
      ready,
    };
  }, [channels, ready]);
  return (
    <MfmChannelsContext.Provider value={byName}>
      <ChannelDirectoryContext.Provider value={directory}>
        {children}
      </ChannelDirectoryContext.Provider>
    </MfmChannelsContext.Provider>
  );
}

export function useMfmChannels() {
  return useContext(MfmChannelsContext);
}

/**
 * 見られるチャンネルの一覧。{@link MfmChannelsProvider} の外（公開プロフィールなど、チャンネルの一覧を
 * 持たない画面）では `null`。その場合、メッセージ・チャンネルのリンクは今までどおり URL のまま描く。
 */
export function useChannelDirectory(): ChannelDirectory | null {
  return useContext(ChannelDirectoryContext);
}

export function ChannelHashtag({ name, className }: { name: string; className?: string }) {
  const channels = useContext(MfmChannelsContext);
  const id = channels.get(name);
  return id ? (
    <Link to="/channels/$channelId" params={{ channelId: id }} className={className}>
      #{name}
    </Link>
  ) : (
    <>#{name}</>
  );
}
