import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { ChannelRow } from "../../../lib/electric/types";

const MfmChannelsContext = createContext<ReadonlyMap<string, string>>(new Map());

/** Share the shell's live channel list with messages and thread replies. */
export function MfmChannelsProvider({
  channels,
  children,
}: {
  channels: readonly Pick<ChannelRow, "id" | "channel_name" | "deleted_at">[];
  children: ReactNode;
}) {
  const byName = useMemo(
    () =>
      new Map(
        channels
          .filter((channel) => channel.deleted_at === null)
          .map((channel) => [channel.channel_name, channel.id]),
      ),
    [channels],
  );
  return <MfmChannelsContext.Provider value={byName}>{children}</MfmChannelsContext.Provider>;
}

export function ChannelHashtag({ name, className }: { name: string; className?: string }) {
  const channels = useContext(MfmChannelsContext);
  const id = channels.get(name);
  return id ? (
    <a href={`/channels/${encodeURIComponent(id)}`} className={className}>
      #{name}
    </a>
  ) : (
    <>#{name}</>
  );
}
