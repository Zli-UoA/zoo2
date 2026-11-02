import type { ReactNode } from "react";
import {
  ActivityPubContent,
  ActivityPubText,
  FederatedAccount,
  type CustomEmoji,
} from "./ActivityPubEmoji";

export interface ProfileDetails {
  id: string;
  user_name: string;
  display_name: string;
  summary?: string;
  emojis?: CustomEmoji[];
  remote: boolean;
}

/** Profile presentation shared by authenticated and anonymous visitors. */
export function ActivityPubProfileDetails({
  account,
  avatar,
  headingLevel = 2,
}: {
  account: ProfileDetails;
  avatar: ReactNode;
  headingLevel?: 1 | 2;
}) {
  const Heading = headingLevel === 1 ? "h1" : "h2";
  return (
    <>
      <div className="flex items-center gap-3">
        {avatar}
        <div className="min-w-0">
          <Heading className="break-words text-lg font-semibold">
            <ActivityPubText
              text={account.display_name || account.user_name}
              emojis={account.emojis}
            />
          </Heading>
          <FederatedAccount
            userName={account.user_name}
            actorId={account.id}
            remote={account.remote}
          />
        </div>
      </div>
      {account.summary ? (
        <div className="mfm-body whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
          <ActivityPubContent text={account.summary} emojis={account.emojis} />
        </div>
      ) : null}
    </>
  );
}
