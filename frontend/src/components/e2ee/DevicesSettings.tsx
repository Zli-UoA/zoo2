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
  RevocationChangedError,
  RevocationConflictError,
  RevocationPendingError,
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
  if (error instanceof RevocationPendingError || error instanceof RevocationChangedError) {
    return "revocationPending";
  }
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
  const fetchDevices = useCallback(() => {
    setDevices(null);
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
  useEffect(fetchDevices, [fetchDevices]);
  if (!devices || devices.step !== "ready") {
    return (
      <DevicesSettingsView
        state={devices ?? { step: "loading" }}
        handlers={{ onRetry: fetchDevices } as DevicesSettingsHandlers}
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
  /** 再開できずに競合の画面を出した失効（「やり直す」で、この失効の記録だけを捨てる）。 */
  const conflictRef = useRef<string | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

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

  /**
   * 端末一覧と検証状態を読み直す。`peers` と `nameOf` は Electric の行が変わるたびに作り直されるので、
   * 失効の途中でも呼ばれる。そのため、一覧のデータだけを差し替え、失効の画面（`revocation`）、実行中の
   * 操作（`busy`）、エラーは変えない（それぞれの操作が自分で戻す）。読み直しに失敗しても、表示中の
   * 画面は残す（最初の読み込みだけ `loadFailed` にする）。
   */
  const load = useCallback(
    async (): Promise<boolean> => {
      const seq = ++loadSeq.current;
      if (!deps) {
        setState({ step: "unavailable" });
        return true;
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
        // あとから始めた読み直しがある。結果はそちらに任せる。
        if (seq !== loadSeq.current) return true;
        const data = {
          devices,
          verifications: verifications.users.map((row) => ({ ...row, name: nameOf(row.userId) })),
          warnings: verifications.warnings,
          unlocked,
          pendingRevocation: pending ? { scope: pending.scope, stage: pending.stage } : null,
          accountUrls,
        };
        setState((current) =>
          current.step === "ready"
            ? { ...current, ...data }
            : {
                step: "ready",
                ...data,
                revocation: null,
                passkeyAvailable: false,
                busy: null,
                error: null,
              },
        );
        return true;
      } catch {
        if (seq !== loadSeq.current) return true;
        setState((current) => (current.step === "ready" ? current : { step: "loadFailed" }));
        return false;
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
      // 読み直してから、操作を終えたことにする（古い一覧のまま押せないように）。
      // 読み直せなければ、表示は残したまま、エラーと読み直しを出す。
      () =>
        load().then((loaded) =>
          ready((current) => ({ ...current, busy: null, error: loaded ? null : "refreshFailed" })),
        ),
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
      if (resumed.kind === "resumed") {
        // 409 などで「やり直す」を押したときも、記録した範囲と端末で始め直す。
        planRef.current = {
          scope: resumed.summary.scope,
          selected: resumed.summary.deleted_devices,
        };
        notesRef.current = notesOf(resumed.summary);
        setPhase({ phase: "confirm", ...notesRef.current });
        return;
      }
      // 2〜5 を SSSS の保存の前に止めた。記録した範囲と端末で最初からやり直す（ページを読み込み
      // 直したあとでも、範囲を端末だけに縮めない）。
      planRef.current = { scope: resumed.scope, selected: resumed.deletedDevices };
      resumeRef.current = false;
    }
    const { scope, selected } = planRef.current;
    // 終わっていない失効があれば、端末を消す前に止める（Worker の `begin` も拒む）。
    if (await revocationStatus(deps)) throw new RevocationPendingError();
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
          // 再開できない失効（別の端末が SSSS を作り直した）の記録は残る。「やり直す」で、この失効だけを
          // 捨ててから始め直す。
          const stale = await revocationStatus(deps).catch(() => null);
          conflictRef.current = stale?.revocation_id ?? null;
          // 始め直すときも、記録した範囲と端末を引き継ぐ（端末だけに縮めない）。
          if (stale) planRef.current = { scope: stale.scope, selected: stale.deleted_devices };
          setPhase({ phase: "conflict" });
          return;
        }
        if (error instanceof RevocationPendingError) {
          // 一覧に戻し、「失効を続ける」を出す。
          setPhase(null, "revocationPending");
          void load();
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
    onRetry: () => {
      // 一覧の画面の読み直し（`refreshFailed`）は、表示を残したまま読み直す。
      ready((current) => ({ ...current, error: null }));
      void load().then((loaded) => {
        if (!loaded) ready((current) => ({ ...current, error: "refreshFailed" }));
      });
    },
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
    onRecoveryKeySaved: () => setPhase({ phase: "confirm", ...notesRef.current }),
    onExecute: () => {
      if (!deps) return;
      const reauth = startReauth(config);
      ready((current) => ({
        ...current,
        busy: "revocation",
        error: null,
        revocation: { phase: "running", stage: "prepared" },
      }));
      let target: string | undefined;
      void (async () => {
        try {
          // 再認証のあいだに画面を離れて別の失効を始めても、その失効を（キーの控えや実行の操作を
          // 待たずに）保存しないよう、実行を始めた失効を固定する。
          target = (await revocationStatus(deps))?.revocation_id;
          const token = await reauth();
          if (!mounted.current) return;
          const result = await continueRevocation(
            deps,
            token,
            (summary) =>
              ready((current) => ({
                ...current,
                revocation: { phase: "running", stage: summary.stage },
              })),
            target,
          );
          setPhase({ phase: "done", result: toResult(result) });
          // 手元の受信用セッションを、新しい版に送り直す（版が変わったので、すべて送り直す）。
          e2ee.requestKeyBackup();
          void load();
        } catch (error) {
          if (error instanceof RevocationChangedError) {
            setPhase(null);
            void load();
            return;
          }
          if (error instanceof RevocationConflictError || error instanceof ServerKeyMismatchError) {
            // 実行した失効だけを捨てる（別の失効に置き換わっていたら捨てない）。
            await abandonRevocation(deps, target).catch(() => undefined);
            conflictRef.current = null;
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
      const stale = conflictRef.current;
      conflictRef.current = null;
      if (!deps || stale === null) {
        setPhase({ phase: "unlock", resume: false });
        return;
      }
      // 再開できなかった失効の記録を、その失効であることを確かめてから捨てる。別の失効に置き換わって
      // いたら捨てずに一覧に戻す（その失効は「失効を続ける」から続ける）。
      ready((current) => ({ ...current, busy: "revocation", error: null }));
      void abandonRevocation(deps, stale).then(
        () => setPhase({ phase: "unlock", resume: false }),
        (error) => {
          setPhase(null, toDevicesError(error, "revocationFailed"));
          void load();
        },
      );
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
