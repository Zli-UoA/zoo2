import { useLiveQuery } from "@tanstack/react-db";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { RevocationScope, RevocationSummary } from "@zoo/crypto-wasm";

import { useUsers } from "../../hooks/useRealtime";
import { createConfiguredApiClient } from "../../lib/api/client";
import type { AppConfig } from "../../lib/config";
import { useCollections } from "../../lib/electric/collections-context";
import { deleteDevice } from "../../lib/e2ee/api";
import { useE2ee } from "../../lib/e2ee/e2ee-context";
import { passkeyPrfSupported, passkeyRpId } from "../../lib/e2ee/passkey";
import { openReauthWindow, reauthenticate } from "../../lib/e2ee/reauth";
import { listDevices, renameDevice } from "../../lib/e2ee/revocationApi";
import {
  RevocationConflictError,
  type RevocationResult,
  abandonRevocation,
  addRevocationPasskey,
  beginRevocation,
  continueRevocation,
  deleteSelectedDevices,
  loadDevices,
  loadVerifications,
  newRevocationRecoveryKey,
  openSecretStorage,
  resumeRevocation,
  revocationStatus,
  reverifyUser,
} from "../../lib/e2ee/revocationController";
import { type E2eeSetupDeps, ServerKeyMismatchError } from "../../lib/e2ee/setupController";
import { accountRevocationUrls } from "../../lib/oidc/accountUrl";
import { getOidcClient } from "../../lib/oidc/session";
import {
  DeviceLimitView,
  type DevicesErrorCode,
  type DevicesSettingsHandlers,
  type DevicesViewState,
  DevicesSettingsView,
  type ResignNoteView,
  type RevocationPhase,
} from "./DevicesSettingsView";
import { toEncryptionError } from "./EncryptionSettings";

/** 再認証して ID トークンを返す。別のウィンドウはクリックの中で同期的に開く。 */
function startReauth(config: AppConfig): () => Promise<string | null> {
  const oidc = getOidcClient(config);
  if (!oidc) return async () => null;
  const popup = openReauthWindow();
  return () => reauthenticate({ oidc, popup });
}

/** 暗号化チャンネル（削除されていないもの）を共有する相手。 */
function useEncryptedPeers(userId: string | null): string[] {
  const { channels, channelUsers } = useCollections();
  const channelsQuery = useLiveQuery((q) => q.from({ channels }));
  const membersQuery = useLiveQuery((q) => q.from({ channelUsers }));
  return useMemo(() => {
    if (!userId) return [];
    const encrypted = new Set(
      (channelsQuery.data ?? [])
        .filter((channel) => channel.encrypted && !channel.deleted_at)
        .map((channel) => channel.id),
    );
    const mine = new Set(
      (membersQuery.data ?? [])
        .filter((row) => row.user_id === userId && encrypted.has(row.channel_id))
        .map((row) => row.channel_id),
    );
    const peers = new Set(
      (membersQuery.data ?? [])
        .filter((row) => mine.has(row.channel_id) && row.user_id !== userId)
        .map((row) => row.user_id),
    );
    return [...peers].sort();
  }, [channelsQuery.data, membersQuery.data, userId]);
}

function toDevicesError(error: unknown, fallback: DevicesErrorCode): DevicesErrorCode {
  if (error instanceof RevocationConflictError) return "mismatch";
  const code = toEncryptionError(error, "saveFailed");
  return code === "saveFailed" ? fallback : code;
}

/** 設定の「端末」タブ（端末一覧、失効、他のユーザーの検証。#362）。 */
export function DevicesSettings() {
  const { deviceLimit } = useE2ee();
  if (deviceLimit) return <DeviceLimit staleIds={deviceLimit.map((d) => d.device_id)} />;
  return <DevicesPanel />;
}

/** 20 台の上限でこの端末を登録できなかったときに、削除する端末を選んでもらう。 */
function DeviceLimit({ staleIds }: { staleIds: string[] }) {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [devices, setDevices] = useState<DevicesViewState | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<DevicesErrorCode | null>(null);
  useEffect(() => {
    void listDevices(client).then(
      (rows) =>
        setDevices({
          step: "ready",
          devices: rows.map((device) => ({
            deviceId: device.device_id,
            displayName: device.display_name,
            createdAt: device.created_at,
            lastSeenAt: device.last_seen_at,
            current: false,
            signed: true,
          })),
          verifications: [],
          warnings: [],
          unlocked: false,
          pendingRevocation: null,
          accountUrls: null,
          revocation: null,
          passkeyAvailable: false,
          busy: null,
          error: null,
        }),
      () => setDevices({ step: "loadFailed" }),
    );
  }, [client]);
  if (!devices || devices.step !== "ready") {
    return (
      <DevicesSettingsView
        state={devices ?? { step: "loading" }}
        handlers={{} as DevicesSettingsHandlers}
      />
    );
  }
  return (
    <DeviceLimitView
      devices={devices.devices}
      staleIds={staleIds}
      busy={busy}
      done={done}
      error={error}
      onDelete={(ids) => {
        setBusy(true);
        setError(null);
        void (async () => {
          try {
            for (const id of ids) await deleteDevice(client, id);
            // 登録は、端末の起動の再試行で行う。
            setDone(true);
          } catch {
            setError("deleteFailed");
          } finally {
            setBusy(false);
          }
        })();
      }}
    />
  );
}

function DevicesPanel() {
  const e2ee = useE2ee();
  const { worker, userId, deviceId } = e2ee;
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const rpId = passkeyRpId(config);
  const [prfSupported, setPrfSupported] = useState(false);
  const [state, setState] = useState<DevicesViewState>({ step: "loading" });
  const peers = useEncryptedPeers(userId);
  const usersQuery = useUsers();
  const names = useMemo(() => {
    const map = new Map<string, string>();
    for (const user of usersQuery.data ?? []) {
      map.set(user.id, user.display_name || user.user_name);
    }
    return map;
  }, [usersQuery.data]);
  const nameOf = useCallback((id: string) => names.get(id) ?? id, [names]);
  /** 失効を始めたときの範囲と、削除した端末（やり直しと、止まった失効の再開に使う）。 */
  const planRef = useRef<{ scope: RevocationScope; selected: string[] }>({
    scope: "device_only",
    selected: [],
  });
  const resumeRef = useRef(false);
  const notesRef = useRef<{ notes: ResignNoteView[]; versionsWithoutKey: string[] }>({
    notes: [],
    versionsWithoutKey: [],
  });
  const loadSeq = useRef(0);

  const deps: E2eeSetupDeps | null = useMemo(
    () => (worker && userId ? { worker, client, userId, rpId } : null),
    [worker, userId, client, rpId],
  );
  const accountUrls = config.oidc ? accountRevocationUrls(config.oidc.issuer) : null;

  useEffect(() => {
    void passkeyPrfSupported().then(setPrfSupported);
  }, []);

  const update = (patch: (current: DevicesViewState) => DevicesViewState) =>
    setState((current) => patch(current));
  const ready = (
    patch: (current: Extract<DevicesViewState, { step: "ready" }>) => DevicesViewState,
  ) => update((current) => (current.step === "ready" ? patch(current) : current));
  const setPhase = (revocation: RevocationPhase | null, error: DevicesErrorCode | null = null) =>
    ready((current) => ({ ...current, revocation, busy: null, error }));

  const load = useCallback(
    async (keepRevocation = false) => {
      const seq = ++loadSeq.current;
      if (!deps) {
        setState({ step: "unavailable" });
        return;
      }
      try {
        const [devices, unlocked, pending] = await Promise.all([
          loadDevices(deps, deviceId),
          deps.worker.request({ op: "hasOwnKeys" }),
          revocationStatus(deps),
        ]);
        const verifications = unlocked
          ? await loadVerifications(deps, peers)
          : { users: [], warnings: [] };
        if (seq !== loadSeq.current) return;
        setState((current) => ({
          step: "ready",
          devices,
          verifications: verifications.users.map((row) => ({ ...row, name: nameOf(row.userId) })),
          warnings: verifications.warnings,
          unlocked,
          pendingRevocation: pending ? { scope: pending.scope, stage: pending.stage } : null,
          accountUrls,
          revocation: keepRevocation && current.step === "ready" ? current.revocation : null,
          passkeyAvailable: false,
          busy: null,
          error: null,
        }));
      } catch {
        if (seq === loadSeq.current) setState({ step: "loadFailed" });
      }
    },
    // accountUrls は config から決まる。
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [deps, deviceId, peers, nameOf],
  );

  useEffect(() => {
    void load();
  }, [load]);

  const notesOf = (summary: RevocationSummary) => ({
    notes: Object.entries(summary.notes).map(([user, notes]) => ({
      userId: user,
      name: nameOf(user),
      notes,
    })),
    versionsWithoutKey: summary.versions_without_key,
  });

  const toResult = (result: RevocationResult) => ({
    scope: result.summary.scope,
    ...notesOf(result.summary),
    unsignedDevices: result.unsignedDevices,
    droppedSessions: result.droppedSessions,
  });

  const run = (busy: "rename" | "delete" | "verify", task: () => Promise<void>) => {
    ready((current) => ({ ...current, busy, error: null }));
    void task().then(
      () => load(),
      (error) =>
        ready((current) => ({
          ...current,
          busy: null,
          error: toDevicesError(
            error,
            busy === "rename"
              ? "renameFailed"
              : busy === "delete"
                ? "deleteFailed"
                : "verifyFailed",
          ),
        })),
    );
  };

  /** SSSS を開けたあと、失効を始める（または止まった失効を続ける）。 */
  const afterUnlock = async () => {
    if (!deps) return;
    if (resumeRef.current) {
      const resumed = await resumeRevocation(deps);
      if (resumed) {
        notesRef.current = notesOf(resumed);
        setPhase({ phase: "confirm", ...notesRef.current });
        return;
      }
      // 2〜5 を SSSS の保存の前に止めた。同じ範囲と端末で最初からやり直す。
      resumeRef.current = false;
    }
    const { scope, selected } = planRef.current;
    await deleteSelectedDevices(deps, selected);
    const summary = await beginRevocation(deps, scope, selected);
    notesRef.current = notesOf(summary);
    if (scope === "full") {
      const recoveryKey = await newRevocationRecoveryKey(deps);
      setPhase({ phase: "recoveryKey", recoveryKey, passkeyAdded: false });
    } else {
      setPhase({ phase: "confirm", ...notesRef.current });
    }
  };

  const unlockWith = (method: { kind: "passkey" } | { kind: "recovery"; recoveryKey: string }) => {
    if (!deps) return;
    ready((current) => ({ ...current, busy: "revocation", error: null }));
    void (async () => {
      try {
        await openSecretStorage(deps, method);
        await afterUnlock();
      } catch (error) {
        if (error instanceof ServerKeyMismatchError) {
          setPhase({ phase: "conflict" });
          return;
        }
        ready((current) => ({
          ...current,
          busy: null,
          error: toDevicesError(error, "revocationFailed"),
        }));
      }
    })();
  };

  const handlers: DevicesSettingsHandlers = {
    onRetry: () => void load(),
    onRename: (id, name) => {
      if (!deps) return;
      run("rename", () => renameDevice(deps.client, id, name));
    },
    onDelete: (id) => {
      if (!deps) return;
      run("delete", () => deleteDevice(deps.client, id));
    },
    onStartRevocation: (selected) => {
      resumeRef.current = false;
      planRef.current = { scope: "device_only", selected };
      setPhase({ phase: "select", scope: "device_only", selected });
    },
    onSelectionChange: (selected) => {
      planRef.current = { ...planRef.current, selected };
      ready((current) =>
        current.revocation?.phase === "select"
          ? { ...current, revocation: { ...current.revocation, selected } }
          : current,
      );
    },
    onScopeChange: (scope) => {
      planRef.current = { ...planRef.current, scope };
      ready((current) =>
        current.revocation?.phase === "select"
          ? { ...current, revocation: { ...current.revocation, scope } }
          : current,
      );
    },
    onConfirmSelection: () => setPhase({ phase: "unlock", resume: false }),
    onResumeRevocation: () => {
      resumeRef.current = true;
      setPhase({ phase: "unlock", resume: true });
    },
    onUnlockWithPasskey: () => unlockWith({ kind: "passkey" }),
    onUnlockWithRecoveryKey: (recoveryKey) => unlockWith({ kind: "recovery", recoveryKey }),
    onAddRevocationPasskey: () => {
      if (!deps) return;
      ready((current) => ({ ...current, busy: "revocation", error: null }));
      void addRevocationPasskey(deps, null).then(
        () =>
          ready((current) =>
            current.revocation?.phase === "recoveryKey"
              ? {
                  ...current,
                  busy: null,
                  revocation: { ...current.revocation, passkeyAdded: true },
                }
              : current,
          ),
        (error) =>
          ready((current) => ({
            ...current,
            busy: null,
            error: toDevicesError(error, "revocationFailed"),
          })),
      );
    },
    onExecute: () => {
      if (!deps) return;
      const reauth = startReauth(config);
      ready((current) => ({
        ...current,
        busy: "revocation",
        error: null,
        revocation: { phase: "running", stage: "prepared" },
      }));
      void (async () => {
        try {
          const token = await reauth();
          const result = await continueRevocation(deps, token, (summary) =>
            ready((current) => ({
              ...current,
              revocation: { phase: "running", stage: summary.stage },
            })),
          );
          setPhase({ phase: "done", result: toResult(result) });
          // 手元の受信用セッションを、新しい版に送り直す（版が変わったので、すべて送り直す）。
          e2ee.requestKeyBackup();
          void load(true);
        } catch (error) {
          if (error instanceof RevocationConflictError || error instanceof ServerKeyMismatchError) {
            await abandonRevocation(deps).catch(() => undefined);
            setPhase({ phase: "conflict" });
            return;
          }
          // 進み具合は暗号ストアにある。同じ画面から、再認証してやり直せる。
          setPhase(
            { phase: "confirm", ...notesRef.current },
            toDevicesError(error, "revocationFailed"),
          );
        }
      })();
    },
    onRestart: () => {
      resumeRef.current = false;
      setPhase({ phase: "unlock", resume: false });
    },
    onCancelRevocation: () => {
      setPhase(null);
      void load();
    },
    onFinish: () => {
      setPhase(null);
      void load();
    },
    onReverify: (target) => {
      if (!deps) return;
      const reauth = startReauth(config);
      run("verify", async () => reverifyUser(deps, target, await reauth()));
    },
  };

  const viewState =
    state.step === "ready" ? { ...state, passkeyAvailable: rpId !== null && prfSupported } : state;
  return <DevicesSettingsView state={viewState} handlers={handlers} />;
}
