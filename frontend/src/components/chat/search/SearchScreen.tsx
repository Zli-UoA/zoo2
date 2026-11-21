import { useLiveQuery } from "@tanstack/react-db";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { LockIcon, SearchIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { useCurrentUser } from "../../../hooks/useCurrentUser";
import { useGlobalSearch } from "../../../hooks/useGlobalSearch";
import { useUsers } from "../../../hooks/useRealtime";
import { useCollections } from "../../../lib/electric/collections-context";
import type { ChannelUserRow } from "../../../lib/electric/types";
import { ScreenHeader } from "../../layout/PaneHeader";
import { Button } from "../../ui/button";
import { ButtonGroup, ButtonGroupItem } from "../../ui/button-group";
import { Input } from "../../ui/input";
import { Spinner } from "../../ui/spinner";
import {
  normalizeSearchQuery,
  parseSearchRouteParams,
  searchKindOf,
  SEARCH_KINDS,
  type SearchKind,
} from "./globalSearch";
import { SearchResultRow } from "./SearchResultRow";

/**
 * 全文検索（`/search?q=&type=`）。語と種類は URL に置き、送信（Enter）でだけ検索する。
 * 古い月の続きは、サーバーが 1 回に見る範囲（12 か月）ごとにボタンで読む。
 */
export function SearchScreen() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const query = useRouterState({
    select: (state) =>
      parseSearchRouteParams(state.location.search as Record<string, unknown>).q ?? "",
  });
  const kind = useRouterState({
    select: (state) =>
      searchKindOf(parseSearchRouteParams(state.location.search as Record<string, unknown>)),
  });
  const [draft, setDraft] = useState(query);
  // 戻る・進むで URL の語が変わったら入力欄も合わせる。
  const [shownQuery, setShownQuery] = useState(query);
  if (shownQuery !== query) {
    setShownQuery(query);
    setDraft(query);
  }
  const search = useGlobalSearch(query, kind);
  const { currentUserId } = useCurrentUser();
  const usersQuery = useUsers();
  const users = useMemo(
    () => new Map((usersQuery.data ?? []).map((row) => [row.id, row])),
    [usersQuery.data],
  );
  const { channelUsers } = useCollections();
  const memberships = useLiveQuery((q) => q.from({ channelUsers }));
  const membersByChannel = useMemo(() => {
    const map = new Map<string, ChannelUserRow[]>();
    for (const member of memberships.data ?? []) {
      const rows = map.get(member.channel_id);
      if (rows) rows.push(member);
      else map.set(member.channel_id, [member]);
    }
    return map;
  }, [memberships.data]);

  const go = (next: { q: string; type: SearchKind }) =>
    void navigate({
      to: "/search",
      search: { q: next.q || undefined, type: next.type === "messages" ? undefined : next.type },
    });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const next = normalizeSearchQuery(draft);
    if (next === query) search.refresh();
    else go({ q: next, type: kind });
  };
  const inputRef = useRef<HTMLInputElement>(null);
  const openedWithoutQuery = useRef(!query);
  useEffect(() => {
    // 語のない状態で開いたら、入力欄へ。
    if (openedWithoutQuery.current) inputRef.current?.focus();
  }, []);

  const firstLoading = search.isLoading && !search.loaded;
  const empty = search.loaded && search.items.length === 0;
  const kindLabel = (value: SearchKind) => t(`globalSearch.kinds.${value}`);

  return (
    <section
      className="relative flex h-full min-h-0 flex-col"
      aria-labelledby="global-search-title"
      aria-busy={search.isLoading}
    >
      <ScreenHeader
        icon={SearchIcon}
        iconClassName="bg-primary text-primary-foreground"
        titleId="global-search-title"
        title={t("globalSearch.title")}
      />
      <div className="space-y-3 border-b p-4">
        <form role="search" onSubmit={submit} className="flex gap-2">
          <Input
            ref={inputRef}
            type="search"
            enterKeyHint="search"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder={t("globalSearch.placeholder")}
            aria-label={t("globalSearch.placeholder")}
            maxLength={600}
            className="min-w-0 flex-1"
          />
          <Button type="submit" size="icon" aria-label={t("globalSearch.title")}>
            <SearchIcon aria-hidden="true" />
          </Button>
        </form>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <ButtonGroup
            value={[kind]}
            onValueChange={(value) => {
              const next = value[0] as SearchKind | undefined;
              if (next && next !== kind) go({ q: query, type: next });
            }}
          >
            {SEARCH_KINDS.map((value) => (
              <ButtonGroupItem
                key={value}
                value={value}
                className="h-8 w-auto px-3 text-sm"
                aria-label={kindLabel(value)}
              >
                {kindLabel(value)}
              </ButtonGroupItem>
            ))}
          </ButtonGroup>
          <p className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
            <LockIcon className="size-3.5 shrink-0" aria-hidden="true" />
            <span>{t("globalSearch.encryptedNote")}</span>
          </p>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {!query ? (
          <div className="flex min-h-72 flex-col items-center justify-center gap-3 p-6 text-center">
            <SearchIcon className="size-8 text-muted-foreground" aria-hidden="true" />
            <p className="max-w-sm text-sm text-muted-foreground">{t("globalSearch.prompt")}</p>
          </div>
        ) : firstLoading ? (
          <Spinner label={t("globalSearch.loading")} className="min-h-72" />
        ) : (
          <>
            {empty && !search.isError ? (
              <div className="flex min-h-48 flex-col items-center justify-end gap-3 p-6 text-center">
                <SearchIcon className="size-8 text-muted-foreground" aria-hidden="true" />
                <p className="max-w-sm text-sm text-muted-foreground">
                  {search.nextCursor && search.searchedUntil
                    ? t("globalSearch.emptyOlder", { month: search.searchedUntil })
                    : t("globalSearch.empty")}
                </p>
              </div>
            ) : null}
            {search.items.length ? (
              <ul className="w-full space-y-3 p-4" aria-label={kindLabel(kind)}>
                {search.items.map((item) => (
                  <SearchResultRow
                    key={item.id}
                    item={item}
                    users={users}
                    members={item.is_dm ? membersByChannel.get(item.channel_id) : undefined}
                    currentUserId={currentUserId}
                  />
                ))}
              </ul>
            ) : null}
            {search.isError ? (
              <div role="alert" className="flex flex-wrap items-center gap-2 px-4 pb-4 text-sm">
                <span className="text-destructive">{t("globalSearch.loadError")}</span>
                <Button variant="outline" size="sm" onClick={search.loadMore}>
                  {t("globalSearch.retry")}
                </Button>
              </div>
            ) : search.nextCursor ? (
              <div
                className={`flex flex-wrap items-center gap-3 px-4 pb-4 ${empty ? "justify-center" : ""}`}
              >
                <Button variant="outline" disabled={search.isLoading} onClick={search.loadMore}>
                  {t(search.isLoading ? "globalSearch.loading" : "globalSearch.searchOlder")}
                </Button>
                {search.searchedUntil && !empty ? (
                  <span className="text-xs text-muted-foreground">
                    {t("globalSearch.searchedUntil", { month: search.searchedUntil })}
                  </span>
                ) : null}
              </div>
            ) : null}
          </>
        )}
      </div>
    </section>
  );
}
