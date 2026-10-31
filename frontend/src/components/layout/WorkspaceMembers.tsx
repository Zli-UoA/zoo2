import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useUsers } from "../../hooks/useRealtime";
import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import { getCurrentUserStore } from "../../lib/currentUser";
import { useCollections } from "../../lib/electric/collections-context";
import type { UserRow } from "../../lib/electric/types";
import { userLabel, sortUsersByLabel } from "../chat/memberCandidates";
import { useLocale } from "../i18n/LocaleProvider";
import { AppAvatar } from "../chat/AppIdentity";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { ConfirmActionDialog } from "../ui/confirm-action-dialog";

export function WorkspaceMembers() {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const { isAdmin, currentUserId } = useCurrentUser();
  const usersQuery = useUsers();
  const [query, setQuery] = useState("");
  const [admins, setAdmins] = useState<Set<string>>(new Set());
  const [protectedIds, setProtectedIds] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState("");
  const [target, setTarget] = useState<{ user: UserRow; isAdmin: boolean } | null>(null);
  const [pending, setPending] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    void client
      .get<{ user_ids: string[]; protected_user_ids?: string[] }>("/api/users/admins", {
        signal: controller.signal,
      })
      .then((value) => {
        if (!controller.signal.aborted) {
          setAdmins(new Set(value.user_ids));
          setProtectedIds(new Set(value.protected_user_ids));
          setError("");
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(t("settings.membersLoadError"));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [client, revision, t]);
  const users = useMemo(
    () =>
      sortUsersByLabel(
        (usersQuery.data ?? []).filter((user) => !user.is_bot),
        locale,
      ).filter((user) =>
        `${userLabel(user)} ${user.user_name}`.toLowerCase().includes(query.trim().toLowerCase()),
      ),
    [usersQuery.data, locale, query],
  );
  async function changeRole() {
    if (!target || pending) return;
    setPending(true);
    setError("");
    try {
      await client.request(`/api/users/${target.user.id}/admin`, {
        method: "PATCH",
        body: { is_admin: !target.isAdmin },
      });
      setAdmins((current) => {
        const next = new Set(current);
        if (target.isAdmin) next.delete(target.user.id);
        else next.add(target.user.id);
        return next;
      });
      window.dispatchEvent(new Event("zoo:admins-changed"));
      if (target.user.id === currentUserId) await getCurrentUserStore(config).refresh();
      setTarget(null);
    } catch (cause) {
      setError(
        cause instanceof ApiError && cause.status === 409
          ? t("settings.lastAdmin")
          : t("common.writeError"),
      );
    } finally {
      setPending(false);
    }
  }
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">{t("settings.membersDescription")}</p>
      <Input
        type="search"
        className="w-full"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder={t("members.search")}
        aria-label={t("members.search")}
      />
      {loading || usersQuery.isLoading ? <p role="status">{t("common.loading")}</p> : null}
      {(error && !target) || usersQuery.isError ? (
        <div role="alert" className="flex items-center gap-3 text-sm text-destructive">
          {error || t("members.loadError")}
          <Button variant="outline" onClick={() => setRevision((value) => value + 1)}>
            {t("offline.retry")}
          </Button>
        </div>
      ) : null}
      <ul className="divide-y rounded-lg border">
        {users.map((user) => (
          <li key={user.id} className="flex flex-wrap items-center gap-3 p-4">
            <AppAvatar label={userLabel(user)} size="sm" />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium">
                {userLabel(user)}
                {user.id === currentUserId ? (
                  <span className="ml-2 text-xs text-muted-foreground">{t("members.you")}</span>
                ) : null}
              </p>
              <p className="truncate text-xs text-muted-foreground">@{user.user_name}</p>
            </div>
            {admins.has(user.id) ? (
              <span className="rounded-md bg-brand-muted px-2 py-1 text-xs text-brand-muted-foreground">
                {t("settings.admin")}
              </span>
            ) : null}
            {isAdmin ? (
              <Button
                variant="outline"
                size="sm"
                disabled={
                  loading ||
                  pending ||
                  !!error ||
                  protectedIds.has(user.id) ||
                  (admins.has(user.id) && admins.size <= 1)
                }
                onClick={() => {
                  setError("");
                  setTarget({ user, isAdmin: admins.has(user.id) });
                }}
              >
                {admins.has(user.id) ? t("settings.removeAdmin") : t("settings.makeAdmin")}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
      {!loading && !usersQuery.isLoading && !users.length ? (
        <p className="text-sm text-muted-foreground">{t("members.usersEmpty")}</p>
      ) : null}
      <ConfirmActionDialog
        open={target !== null}
        onOpenChange={(open) => {
          if (!open) {
            setTarget(null);
            setError("");
          }
        }}
        title={target?.isAdmin ? t("settings.removeAdmin") : t("settings.makeAdmin")}
        description={t("settings.roleConfirm", {
          name: target ? userLabel(target.user) : "",
          role: target?.isAdmin ? t("settings.member") : t("settings.admin"),
        })}
        confirmLabel={target?.isAdmin ? t("settings.removeAdmin") : t("settings.makeAdmin")}
        onConfirm={changeRole}
        pending={pending}
        error={error}
      />
    </div>
  );
}
