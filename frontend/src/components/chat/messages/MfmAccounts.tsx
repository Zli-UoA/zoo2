import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { AuthZAccount } from "../../../lib/emoji/accounts";
import { Avatar, AvatarFallback, AvatarImage } from "../../ui/avatar";

export type MfmAccount = AuthZAccount;
const AccountsContext = createContext<ReadonlyMap<string, MfmAccount>>(new Map());

/** Reuse the shell's live directory; each message does not fetch a user list. */
export function MfmAccountsProvider({
  accounts,
  children,
}: {
  accounts: readonly MfmAccount[];
  children: ReactNode;
}) {
  const byName = useMemo(
    () => new Map(accounts.map((account) => [account.username, account])),
    [accounts],
  );
  return <AccountsContext.Provider value={byName}>{children}</AccountsContext.Provider>;
}

export function useMfmAccounts() {
  return useContext(AccountsContext);
}

export function AccountGlyph({ name, mention = false }: { name: string; mention?: boolean }) {
  const account = useMfmAccounts().get(name);
  if (!account || ["channel", "here"].includes(name.toLowerCase()))
    return <>{mention ? `@${name}` : `:@${name}:`}</>;
  const label = account.display_name || account.username;
  const url = account.avatar_url;
  const safeUrl = url && (/^https?:\/\//i.test(url) || /^\/(?!\/)/.test(url)) ? url : undefined;
  return (
    <span
      className="inline-flex max-w-full items-baseline gap-0.5 align-baseline"
      title={`@${account.username}`}
    >
      {mention ? "@" : null}
      <span
        role="img"
        aria-label={mention ? undefined : `:@${name}:`}
        aria-hidden={mention || undefined}
        className="inline-flex self-center"
      >
        <Avatar className="inline-flex size-[1.15em] align-middle after:border-0">
          <AvatarImage src={safeUrl} alt="" />
          <AvatarFallback className="text-[0.7em]">{Array.from(label)[0] ?? "?"}</AvatarFallback>
        </Avatar>
      </span>
      {mention ? <span className="[overflow-wrap:anywhere]">{label}</span> : null}
    </span>
  );
}
