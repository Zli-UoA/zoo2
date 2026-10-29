import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { cn } from "cn";
import { PlusIcon, SearchIcon, XIcon } from "lucide-react";
import {
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
} from "react";

import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { localizeApiError } from "../../lib/i18n/apiError";
import { createOrGetDm } from "../../lib/api/dms";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import type { UserRow } from "../../lib/electric/types";
import { useLocale } from "../i18n/LocaleProvider";
import { useShell } from "../layout/shell-context";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Button } from "../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";
import { inputClassName } from "../ui/input-styles";
import { ScrollArea } from "../ui/scroll-area";
import { DM_MAX_MEMBERS, DM_MAX_TARGETS, filterDmCandidates, waitForDmAvailability } from "./dm";
import { initials } from "./format";
import { sortUsersByLabel, userLabel } from "./memberCandidates";

export function CreateDmDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const shell = useShell();
  const navigate = useNavigate();
  const collections = useCollections();
  const client = useMemo(() => createConfiguredApiClient(collections.config), [collections]);
  const usersQuery = useUsers();
  const { currentUserId, isLoading: currentUserLoading } = useCurrentUser();
  const [query, setQuery] = useState("");
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [limitReached, setLimitReached] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [createdChannelId, setCreatedChannelId] = useState<string | null>(null);
  const [syncPending, setSyncPending] = useState(false);
  const [errorCause, setErrorCause] = useState<{ cause: unknown } | null>(null);
  const submittingRef = useRef(false);
  const listId = useId();

  const users = usersQuery.data ?? [];
  const usersById = useMemo(() => new Map(users.map((user) => [user.id, user])), [users]);
  const sortedUsers = useMemo(() => sortUsersByLabel(users, locale), [users, locale]);
  const candidates = useMemo(
    () => filterDmCandidates({ users: sortedUsers, currentUserId, selectedIds, query }),
    [currentUserId, query, selectedIds, sortedUsers],
  );
  const selectedUsers = selectedIds.map((id) => ({
    id,
    user: usersById.get(id) as UserRow | undefined,
  }));
  const resolvedIndex = candidates.length === 0 ? -1 : Math.min(activeIndex, candidates.length - 1);
  const activeCandidate = resolvedIndex >= 0 ? candidates[resolvedIndex] : undefined;
  const inputBusy = submitting || currentUserLoading || createdChannelId !== null;
  const canSubmit = createdChannelId
    ? !submitting && currentUserId !== null
    : selectedIds.length > 0 && !inputBusy && currentUserId !== null;

  function reset() {
    setQuery("");
    setSelectedIds([]);
    setActiveIndex(0);
    setLimitReached(false);
    setCreatedChannelId(null);
    setSyncPending(false);
    setSubmitting(false);
    setErrorCause(null);
    submittingRef.current = false;
  }

  function handleOpenChange(next: boolean) {
    if (!next && submittingRef.current) {
      return;
    }
    if (!next) {
      reset();
    }
    onOpenChange(next);
  }

  function addUser(user: UserRow) {
    if (inputBusy) {
      return;
    }
    if (selectedIds.length >= DM_MAX_TARGETS) {
      setLimitReached(true);
      return;
    }
    setSelectedIds((ids) => [...ids, user.id]);
    setQuery("");
    setActiveIndex(0);
    setLimitReached(false);
    setErrorCause(null);
  }

  function removeUser(userId: string) {
    setSelectedIds((ids) => ids.filter((id) => id !== userId));
    setLimitReached(false);
    setErrorCause(null);
  }

  function handleSearchKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) {
      return;
    }
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        setActiveIndex((index) => Math.min(index + 1, candidates.length - 1));
        break;
      case "ArrowUp":
        event.preventDefault();
        setActiveIndex((index) => Math.max(index - 1, 0));
        break;
      case "Home":
        event.preventDefault();
        setActiveIndex(candidates.length > 0 ? 0 : -1);
        break;
      case "End":
        event.preventDefault();
        setActiveIndex(candidates.length - 1);
        break;
      case "Enter": {
        if (activeCandidate) {
          event.preventDefault();
          addUser(activeCandidate);
        }
        break;
      }
      case "Escape":
        if (query) {
          event.preventDefault();
          event.stopPropagation();
          setQuery("");
          setActiveIndex(0);
        }
        break;
    }
  }

  function handleCandidateMouseDown(event: MouseEvent<HTMLLIElement>) {
    event.preventDefault();
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!canSubmit || !currentUserId || submittingRef.current) {
      return;
    }
    submittingRef.current = true;
    setSubmitting(true);
    setErrorCause(null);
    try {
      let channelId = createdChannelId;
      if (!channelId) {
        const { data } = await createOrGetDm(client, selectedIds);
        channelId = data.id;
        setCreatedChannelId(channelId);
      }
      setSyncPending(false);
      const ready = await waitForDmAvailability(collections, channelId, currentUserId);
      if (!ready) {
        setSyncPending(true);
        setSubmitting(false);
        submittingRef.current = false;
        return;
      }
      reset();
      onOpenChange(false);
      shell.showChat();
      await navigate({ to: "/channels/$channelId", params: { channelId } });
    } catch (cause) {
      submittingRef.current = false;
      setSubmitting(false);
      setErrorCause({ cause });
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        className="max-h-[85dvh] overflow-y-auto sm:max-w-md"
        showCloseButton={!submitting}
      >
        <DialogHeader>
          <DialogTitle>{t("dms.createTitle")}</DialogTitle>
          <DialogDescription>{t("dms.createDescription")}</DialogDescription>
        </DialogHeader>

        <form className="flex flex-col gap-3" onSubmit={submit}>
          <div className="relative min-w-0">
            <SearchIcon
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
            />
            <input
              type="search"
              className={cn(
                inputClassName(),
                "w-full pr-8 pl-8 [&::-webkit-search-cancel-button]:appearance-none",
              )}
              placeholder={t("dms.search")}
              aria-label={t("dms.search")}
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setActiveIndex(0);
              }}
              onKeyDown={handleSearchKeyDown}
              role="combobox"
              aria-expanded={candidates.length > 0}
              aria-controls={candidates.length > 0 ? listId : undefined}
              aria-autocomplete="list"
              aria-activedescendant={
                activeCandidate ? `${listId}-option-${activeCandidate.id}` : undefined
              }
              autoComplete="off"
              spellCheck={false}
              autoFocus
              disabled={inputBusy}
            />
            {query ? (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="absolute top-1/2 right-0 size-9 -translate-y-1/2"
                aria-label={t("dms.searchClear")}
                title={t("dms.searchClear")}
                disabled={inputBusy}
                onClick={() => {
                  setQuery("");
                  setActiveIndex(0);
                }}
              >
                <XIcon />
              </Button>
            ) : null}
          </div>

          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-muted-foreground">{t("dms.selected")}</span>
              <span role="status" aria-live="polite" className="text-xs tabular-nums">
                {t("dms.selectedCount", {
                  count: selectedIds.length,
                  max: DM_MAX_TARGETS,
                })}
              </span>
            </div>
            {selectedUsers.length > 0 ? (
              <ul className="flex flex-wrap gap-1.5" aria-label={t("dms.selected")}>
                {selectedUsers.map(({ id, user }) => {
                  const label = user ? userLabel(user) : id;
                  return (
                    <li key={id}>
                      <button
                        type="button"
                        className="flex min-h-11 items-center gap-1.5 rounded-full border bg-muted/40 py-1 pr-2 pl-1 text-sm outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50"
                        aria-label={t("dms.remove", { name: label })}
                        title={t("dms.remove", { name: label })}
                        disabled={inputBusy}
                        onClick={() => removeUser(id)}
                      >
                        <Avatar size="sm" aria-hidden="true">
                          <AvatarFallback>{initials(label)}</AvatarFallback>
                        </Avatar>
                        <span className="max-w-40 truncate">{label}</span>
                        <XIcon
                          className="size-3.5 shrink-0 text-muted-foreground"
                          aria-hidden="true"
                        />
                      </button>
                    </li>
                  );
                })}
              </ul>
            ) : (
              <p className="text-xs text-muted-foreground">{t("dms.selectHelp")}</p>
            )}
            <p className="text-xs text-muted-foreground">
              {t("dms.memberLimit", { max: DM_MAX_MEMBERS })}
            </p>
            {limitReached ? (
              <p role="status" className="text-xs text-destructive">
                {t("dms.limitReached")}
              </p>
            ) : null}
          </div>

          <div className="h-64 min-h-0 min-w-0">
            {usersQuery.isLoading ? (
              <p className="flex h-full items-center justify-center px-2 text-center text-sm text-muted-foreground">
                {t("dms.usersLoading")}
              </p>
            ) : usersQuery.isError ? (
              <p className="flex h-full items-center justify-center px-2 text-center text-sm text-destructive">
                {t("dms.usersError")}
              </p>
            ) : users.length === 0 ? (
              <p className="flex h-full items-center justify-center px-2 text-center text-sm text-muted-foreground">
                {t("dms.usersEmpty")}
              </p>
            ) : candidates.length === 0 ? (
              <p className="flex h-full items-center justify-center px-2 text-center text-sm text-muted-foreground">
                {query.trim() ? t("dms.noResults") : t("dms.candidatesEmpty")}
              </p>
            ) : (
              <ScrollArea className="h-full">
                <ul id={listId} role="listbox" aria-label={t("dms.candidates")} className="pr-2">
                  {candidates.map((user, index) => {
                    const label = userLabel(user);
                    const active = index === resolvedIndex;
                    return (
                      <li
                        key={user.id}
                        id={`${listId}-option-${user.id}`}
                        role="option"
                        aria-selected={active}
                        className={cn(
                          "flex min-h-11 cursor-pointer items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm",
                          active ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
                        )}
                        onMouseDown={handleCandidateMouseDown}
                        onMouseEnter={() => setActiveIndex(index)}
                        onClick={() => addUser(user)}
                      >
                        <Avatar size="sm" aria-hidden="true">
                          <AvatarFallback>{initials(label)}</AvatarFallback>
                        </Avatar>
                        <span className="min-w-0 flex-1 truncate">{label}</span>
                        {label !== user.user_name ? (
                          <span className="max-w-[40%] shrink-0 truncate text-xs text-muted-foreground">
                            @{user.user_name}
                          </span>
                        ) : null}
                        <PlusIcon
                          className="size-4 shrink-0 text-muted-foreground"
                          aria-hidden="true"
                        />
                      </li>
                    );
                  })}
                </ul>
              </ScrollArea>
            )}
          </div>

          {syncPending ? (
            <p role="status" className="text-xs text-destructive">
              {t("dms.syncPending")}
            </p>
          ) : null}
          {errorCause ? (
            <p className="text-xs text-destructive">
              {t("common.writeErrorDetail", {
                error: localizeApiError(errorCause.cause, t),
              })}
            </p>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              className="min-h-11"
              disabled={submitting}
              onClick={() => handleOpenChange(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button type="submit" className="min-h-11" disabled={!canSubmit}>
              {submitting
                ? createdChannelId
                  ? t("dms.opening")
                  : t("dms.creating")
                : createdChannelId
                  ? t("dms.openCreated")
                  : t("dms.create")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
