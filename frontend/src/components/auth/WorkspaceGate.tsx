import { useEffect, useState, type FormEvent, type ReactNode } from "react";

import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import { getOidcClient } from "../../lib/oidc/session";
import { resolveConfig } from "../../lib/config";
import { getSelectedTenantId, saveSelectedTenantId, switchTenant } from "../../lib/tenant";
import { Button } from "../ui/button";
import { ShellSkeleton } from "../layout/ShellSkeleton";

interface Workspace {
  id: string;
  name: string;
}

function errorMessage(error: unknown): string {
  if (error instanceof ApiError && error.status === 403)
    return "メンバーの追加には管理者権限が必要です。";
  if (error instanceof ApiError && error.status === 404)
    return "ユーザーが見つかりません。ユーザー名を確認してください。";
  return "操作に失敗しました。時間をおいて再度お試しください。";
}

export function WorkspaceGate({ children }: { children: ReactNode }) {
  const [workspaces, setWorkspaces] = useState<Workspace[] | null>(null);
  const [userName, setUserName] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [mode, setMode] = useState<"create" | "invite" | null>(null);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const config = resolveConfig();
    if (config.shapeMode === "direct") {
      setWorkspaces([]);
      return;
    }
    const client = createConfiguredApiClient(config);
    void Promise.all([
      client.get<Workspace[]>("/api/tenants"),
      client.get<{ user_name: string }>("/api/users/me"),
    ])
      .then(([items, me]) => {
        if (cancelled) return;
        const current = items.find((item) => item.id === getSelectedTenantId()) ?? items[0];
        if (current) saveSelectedTenantId(current.id);
        setUserName(me.user_name);
        setSelected(current?.id ?? null);
        setWorkspaces(items);
        if (!current) setMode("create");
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(errorMessage(cause));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const client = createConfiguredApiClient(resolveConfig());
      if (mode === "create") {
        const result = await client.post<Workspace>("/api/tenants", { name: value });
        switchTenant(result.data.id);
      } else {
        // Existing accounts are added by exact username; the API enforces workspace admin permission.
        await client.request("/api/tenants/members", {
          method: "POST",
          body: { user_name: value },
        });
        setNotice("メンバーを追加しました。");
        setValue("");
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  if (typeof window !== "undefined" && resolveConfig().shapeMode === "direct") return children;
  if (!workspaces)
    return error ? (
      <p role="alert" className="p-6">
        {error}
        <Button onClick={() => window.location.reload()}>再試行</Button>
      </p>
    ) : (
      <ShellSkeleton />
    );

  return (
    <div className="flex h-dvh flex-col bg-background text-foreground">
      <header className="flex flex-wrap items-center gap-3 border-b p-2">
        {userName ? (
          <span className="text-sm text-muted-foreground" aria-label="自分のユーザー名">
            @{userName}
          </span>
        ) : null}
        {!selected && resolveConfig().oidc ? (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              void getOidcClient(resolveConfig())?.logout();
            }}
          >
            ログアウト
          </Button>
        ) : null}
        <label className="flex items-center gap-2 text-sm">
          ワークスペース
          <select
            aria-label="ワークスペース"
            value={selected ?? ""}
            onChange={(event) => switchTenant(event.target.value)}
            disabled={!selected || busy}
            className="max-w-56 rounded border bg-background p-1"
          >
            {!selected ? <option value="">未所属</option> : null}
            {workspaces.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name}
              </option>
            ))}
          </select>
        </label>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => {
            setMode("create");
            setValue("");
            setError(null);
            setNotice(null);
          }}
        >
          新規作成
        </Button>
        {selected ? (
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => {
              setMode("invite");
              setValue("");
              setError(null);
              setNotice(null);
            }}
          >
            メンバーを追加
          </Button>
        ) : null}
      </header>
      {mode ? (
        <form
          onSubmit={(event) => {
            void submit(event);
          }}
          className="flex flex-wrap items-center gap-2 border-b p-3"
        >
          <label className="flex items-center gap-2 text-sm">
            {mode === "create" ? "ワークスペース名" : "追加するユーザー名"}
            <input
              required
              maxLength={mode === "create" ? 100 : 80}
              value={value}
              onChange={(event) => setValue(event.target.value)}
              disabled={busy}
              className="rounded border bg-background p-2"
            />
          </label>
          <Button type="submit" size="sm" disabled={busy}>
            {mode === "create" ? "作成" : "追加"}
          </Button>
          {selected ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                setMode(null);
                setError(null);
                setNotice(null);
              }}
            >
              閉じる
            </Button>
          ) : null}
          {mode === "invite" ? (
            <p className="text-sm text-muted-foreground">
              Zoo にログイン済みのユーザーを管理者が追加できます。
            </p>
          ) : null}
        </form>
      ) : null}
      {error ? (
        <p role="alert" className="p-3 text-destructive">
          {error}
        </p>
      ) : null}
      {notice ? (
        <p role="status" className="p-3">
          {notice}
        </p>
      ) : null}
      {selected ? (
        <div className="min-h-0 flex-1">{children}</div>
      ) : (
        <p className="p-6">ワークスペースを作成するか、管理者にメンバー追加を依頼してください。</p>
      )}
    </div>
  );
}
