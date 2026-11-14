import { readFileSync } from "node:fs";

import {
  BackupDecryptionKey,
  DeviceAccount,
  OutboundSession,
  OwnSecrets,
  RecoveryKey,
  SecretStorage,
  initSync,
  verifyDeviceKeys,
  type JsonObject,
} from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it } from "vite-plus/test";

import { ApiError, type ApiClient, type RequestOptions } from "../api/client";
import type { CryptoWorker } from "./cryptoWorker";
import { CryptoOperations } from "./operations";
import type { CredentialGetter } from "./passkey";
import type { CryptoOperation, KeysQueryResponse } from "./protocol";
import { REAUTH_HEADER } from "./secretStorageApi";
import {
  ReauthRequiredError,
  ServerKeyMismatchError,
  SetupConflictError,
  addSetupPasskey,
  beginSetup,
  addPasskeySlot,
  completeSetup,
  hasUnlockedSecrets,
  loadE2eeStatus,
  runStartupCheck,
  uploadKeyBackup,
  importKeyBackup,
  saveRemaining,
  unlock,
  type E2eeSetupDeps,
} from "./setupController";
import { MemoryCryptoBackend } from "./store/backend";
import type { WrappingKeyStore } from "./store/storageKey";

/** 端末鍵だけの `keys/query` の応答（クロス署名鍵がないので、送信端末は検証済みにならない）。 */
function keysQueryOf(deviceKeys: Record<string, Record<string, JsonObject>>): KeysQueryResponse {
  return {
    device_keys: deviceKeys,
    master_keys: {},
    self_signing_keys: {},
    user_signing_keys: {},
    tofu_pins: {},
    signatures: [],
  };
}

beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

const USER = "00000000-0000-4000-8000-00000000000a";
const RP_ID = "account.zli.works";
const TOKEN = "reauth-id-token";

type Json = Record<string, unknown>;

/**
 * docs/e2ee.md の API を、初回設定と解錠に要る範囲だけ持つサーバー。`fail` で、指定した保存の
 * 直前（反映しない）か直後（反映したが応答をなくす）に失敗させる。
 */
class FakeServer {
  ssss: Json | null = null;
  revision = 0;
  master: Json | null = null;
  selfSigning: Json | null = null;
  userSigning: Json | null = null;
  signatures: Json[] = [];
  versions: Json[] = [];
  backupSessions: Json[] = [];
  /** `GET /api/e2ee/key_backup/keys` の 1 ページの件数（実際のサーバーは 1000）。 */
  backupPageSize = 1000;
  fail: { path: string; when: "before" | "after" } | null = null;
  requests: { method: string; path: string; reauth: boolean }[] = [];

  client(): ApiClient {
    const handle = async (path: string, options: RequestOptions = {}) => {
      const method = options.method ?? "GET";
      const reauth = options.headers?.[REAUTH_HEADER] === TOKEN;
      this.requests.push({ method, path, reauth });
      const fail = this.fail?.path === `${method} ${path}` ? this.fail : null;
      if (fail?.when === "before") {
        this.fail = null;
        throw new TypeError("network error");
      }
      if (path === "/api/e2ee/keys/query" && this.beforeKeysQuery) {
        const hook = this.beforeKeysQuery;
        this.beforeKeysQuery = null;
        await hook();
      }
      const result = this.respond(method, path, options.body as Json, reauth);
      if (fail?.when === "after") {
        this.fail = null;
        throw new TypeError("network error");
      }
      return result;
    };
    const envelope = async (path: string, options: RequestOptions) => ({
      data: await handle(path, options),
      txid: null,
    });
    return {
      request: (path, options) => handle(path, options) as never,
      requestWithTx: (path, options) => envelope(path, options ?? {}) as never,
      get: (path, options) => handle(path, { ...options, method: "GET" }) as never,
      post: (path, body, options) => envelope(path, { ...options, method: "POST", body }) as never,
      patch: (path, body, options) =>
        envelope(path, { ...options, method: "PATCH", body }) as never,
      delete: (path, options) => envelope(path, { ...options, method: "DELETE" }) as never,
    };
  }

  private error(status: number, code: string, extra: Json = {}): never {
    throw new ApiError(status, code, { error: code, code, ...extra });
  }

  /** 実際のサーバーと同じく、前提にした既定の SSSS 鍵を照合する（省略したら照合しない）。 */
  private checkExpectedDefaultKey(body: Json) {
    if (body.expected_default_key_id === undefined) return;
    const current = (this.ssss?.default_key_id as string | undefined) ?? null;
    if (body.expected_default_key_id !== current) {
      this.error(409, "E2EE_SSSS_CONFLICT", { default_key_id: current });
    }
  }

  /** 次の `keys/query` の応答を返す前に、`hook` を待つ（読み直しと保存の間の競合を作るため）。 */
  beforeKeysQuery: (() => Promise<void>) | null = null;

  private requireReauth(reauth: boolean) {
    if (!reauth) this.error(403, "E2EE_REAUTH_REQUIRED");
  }

  private respond(method: string, fullPath: string, body: Json, reauth: boolean): unknown {
    const [path, search] = fullPath.split("?");
    const params = new URLSearchParams(search);
    const route = `${method} ${path!.replace(/\/[0-9a-f-]{36}(\/used)?$/, "/:id$1")}`;
    switch (route) {
      case "GET /api/e2ee/secret_storage":
        return this.ssss
          ? { ...this.ssss, secrets_revision: this.revision }
          : { default_key_id: null, keys: {}, secrets: {}, secrets_revision: null, slots: [] };
      case "POST /api/e2ee/secret_storage/keys": {
        this.requireReauth(reauth);
        const current = (this.ssss?.default_key_id as string | undefined) ?? null;
        if (
          body.expected_default_key_id !== current ||
          body.expected_secrets_revision !== (this.ssss ? this.revision : null)
        ) {
          this.error(409, "E2EE_SSSS_CONFLICT", {
            default_key_id: current,
            secrets_revision: this.ssss ? this.revision : null,
          });
        }
        const keyId = body.key_id as string;
        const secrets = Object.fromEntries(
          Object.entries(body.secrets as Json).map(([name, secret]) => [
            name,
            { ...(secret as Json), key_id: keyId },
          ]),
        );
        this.ssss = {
          default_key_id: keyId,
          keys: { [keyId]: body.key },
          secrets,
          slots: (body.slots as Json[]).map((slot) => ({
            ...slot,
            key_id: keyId,
            created_at: "2026-11-13T00:00:00Z",
            last_used_at: null,
          })),
        };
        this.revision += 1;
        return { secrets_revision: this.revision };
      }
      case "POST /api/e2ee/secret_storage/slots/:id/used":
        return {};
      case "POST /api/e2ee/secret_storage/slots": {
        if (!this.ssss || body.key_id !== this.ssss.default_key_id) {
          this.error(409, "E2EE_SSSS_CONFLICT");
        }
        if (body.label === null) this.error(422, "label が null です");
        const slots = this.ssss.slots as Json[];
        const replaced = slots.filter(
          (slot) => body.kind === "passkey" && slot.credential_id === body.credential_id,
        );
        if (replaced.length > 0) this.requireReauth(reauth);
        this.ssss.slots = [
          ...slots.filter((slot) => !replaced.includes(slot)),
          { ...body, created_at: "2026-11-13T00:00:00Z", last_used_at: null },
        ];
        return {};
      }
      case "POST /api/e2ee/keys/query": {
        const own = (key: Json | null) => (key ? { [USER]: key } : {});
        return {
          device_keys: {},
          master_keys: own(this.master),
          self_signing_keys: own(this.selfSigning),
          user_signing_keys: own(this.userSigning),
          signatures: this.signatures,
          tofu_pins: {},
        };
      }
      case "GET /api/e2ee/key_backup/versions":
        return { versions: this.versions };
      case "GET /api/e2ee/key_backup/versions/current": {
        const current = this.versions.at(-1);
        if (!current || current.deleted) this.error(404, "NOT_FOUND");
        return { ...current, count: this.backupSessions.length };
      }
      case "GET /api/e2ee/key_backup/keys": {
        const version = this.versions.find((v) => v.version === params.get("version"));
        if (!version || version.deleted) this.error(404, "NOT_FOUND");
        const key = (row: Json) => JSON.stringify([row.channel_id, row.sender_key, row.session_id]);
        const rows = [...this.backupSessions].sort((a, b) => key(a).localeCompare(key(b)));
        const start = Number(params.get("after") ?? 0);
        const end = start + this.backupPageSize;
        return {
          sessions: rows.slice(start, end),
          next: end < rows.length ? String(end) : null,
        };
      }
      case "POST /api/e2ee/key_backup/keys": {
        const current = this.versions.at(-1);
        if (!current || params.get("version") !== current.version) {
          this.error(409, "E2EE_WRONG_BACKUP_VERSION");
        }
        if (params.get("secrets_revision") !== String(this.revision)) {
          this.error(409, "E2EE_SSSS_CONFLICT");
        }
        this.backupSessions.push(...(body.sessions as Json[]));
        return { count: this.backupSessions.length };
      }
      case "POST /api/e2ee/cross_signing/keys": {
        this.requireReauth(reauth);
        this.checkExpectedDefaultKey(body);
        const keyOf = (key: Json | null) =>
          key ? Object.values(key.keys as Record<string, string>)[0] : null;
        const expected = body.expected_keys as Json;
        if (
          expected.master !== keyOf(this.master) ||
          expected.self_signing !== keyOf(this.selfSigning) ||
          expected.user_signing !== keyOf(this.userSigning)
        ) {
          this.error(409, "E2EE_CROSS_SIGNING_CONFLICT");
        }
        if (body.master_key) this.master = body.master_key as Json;
        if (body.self_signing_key) this.selfSigning = body.self_signing_key as Json;
        if (body.user_signing_key) this.userSigning = body.user_signing_key as Json;
        return {};
      }
      case "POST /api/e2ee/cross_signing/signatures":
        for (const signature of body.signatures as Json[]) {
          this.signatures.push({ signer_user_id: USER, ...signature });
        }
        return { count: (body.signatures as Json[]).length };
      case "POST /api/e2ee/key_backup/versions": {
        this.checkExpectedDefaultKey(body);
        const current = this.versions.at(-1);
        const authData = body.auth_data as Json;
        if (current && (current.auth_data as Json).public_key === authData.public_key) {
          return { version: current.version };
        }
        if (body.expected_current_version !== ((current?.version as string | undefined) ?? null)) {
          this.error(409, "E2EE_WRONG_BACKUP_VERSION");
        }
        if (current) this.requireReauth(reauth);
        const version = String(this.versions.length + 1);
        this.versions.push({
          version,
          algorithm: body.algorithm,
          auth_data: authData,
          deleted: false,
        });
        return { version };
      }
      default:
        throw new Error(`unexpected request ${route}`);
    }
  }
}

function memoryKeys(): WrappingKeyStore {
  const keys = new Map<string, CryptoKey>();
  return {
    get: async (name) => keys.get(name),
    put: async (name, key) => {
      keys.set(name, key);
    },
    delete: async (name) => {
      keys.delete(name);
    },
  };
}

/** 1 つの端末。`restart` で Worker のメモリー（マスター鍵と SSSS 鍵）を失い、暗号ストアは残る。 */
class Device {
  readonly backend = new MemoryCryptoBackend();
  private readonly keys = memoryKeys();
  private operations!: CryptoOperations;
  worker!: CryptoWorker;

  constructor(private readonly namespace: string) {}

  async start(): Promise<void> {
    this.operations = new CryptoOperations(
      { open: async () => this.backend, remove: async () => {} },
      this.keys,
    );
    const operations = this.operations;
    this.worker = {
      request: (operation) => operations.run(operation as CryptoOperation) as never,
      terminate: () => {},
    };
    await this.worker.request({ op: "open", namespace: this.namespace, userId: USER });
  }

  deps(server: FakeServer, getCredential?: CredentialGetter): E2eeSetupDeps {
    return {
      worker: this.worker,
      client: server.client(),
      userId: USER,
      rpId: RP_ID,
      getCredential,
    };
  }
}

async function device(name: string): Promise<Device> {
  const created = new Device(name);
  await created.start();
  return created;
}

function deviceSignatureCount(server: FakeServer): number {
  const selfSigningKey = Object.keys(server.selfSigning?.keys as Record<string, string>)[0];
  return server.signatures.filter((row) => row.signer_key_id === selfSigningKey).length;
}

/**
 * 仮想の認証器。credential ID ごとの秘密で、PRF の salt から決まった 32 バイトを返す
 * （WebAuthn の PRF と同じく、同じパスキーと salt なら同じ出力）。
 */
function virtualAuthenticator(credentialId = "Y3JlZGVudGlhbC0x") {
  const getter: CredentialGetter = async (options) => {
    const publicKey = options.publicKey!;
    const prf = (publicKey.extensions as { prf?: Json }).prf as {
      eval?: { first: Uint8Array };
      evalByCredential?: Record<string, { first: Uint8Array }>;
    };
    const salt = prf.eval?.first ?? prf.evalByCredential?.[credentialId]?.first;
    if (!salt) throw Object.assign(new Error("no credential"), { name: "NotAllowedError" });
    const secret = new TextEncoder().encode(`secret:${credentialId}`);
    const key = await crypto.subtle.importKey(
      "raw",
      secret,
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["sign"],
    );
    const first = await crypto.subtle.sign("HMAC", key, salt as Uint8Array<ArrayBuffer>);
    const rawId = Uint8Array.from(atob(credentialId), (c) => c.charCodeAt(0)).buffer;
    return {
      type: "public-key",
      rawId,
      getClientExtensionResults: () => ({ prf: { results: { first } } }),
    } as unknown as Credential;
  };
  return getter;
}

describe("初回設定と解錠", () => {
  it("初回設定のあと、別の端末でリカバリーキーで解錠できる", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    expect(await loadE2eeStatus(deps)).toEqual({ kind: "notSetUp" });

    const { recoveryKey, keyId } = await beginSetup(deps, null);
    expect(recoveryKey.replace(/ /g, "")).toHaveLength(48);
    await completeSetup(deps, keyId, TOKEN);
    expect(server.ssss?.default_key_id).toBe(keyId);
    expect(server.master).not.toBeNull();
    expect(server.versions).toHaveLength(1);
    expect(deviceSignatureCount(server)).toBe(1);
    expect((await loadE2eeStatus(deps)).kind).toBe("unlocked");

    const second = await device("second");
    const secondDeps = second.deps(server);
    const status = await loadE2eeStatus(secondDeps);
    expect(status).toMatchObject({ kind: "locked", reason: "new" });
    if (status.kind !== "locked") throw new Error("unreachable");
    const result = await unlock(secondDeps, status.storage, { kind: "recovery", recoveryKey });
    expect(result.needsReauth).toBe(false);
    expect(deviceSignatureCount(server)).toBe(2);
    expect((await loadE2eeStatus(secondDeps)).kind).toBe("unlocked");
  });

  it("形式の違うリカバリーキーと、違うリカバリーキーを区別する", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const { keyId } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);
    const other = await device("other");
    const status = await loadE2eeStatus(other.deps(server));
    if (status.kind !== "locked") throw new Error("unreachable");
    const storage = status;
    await expect(
      unlock(other.deps(server), storage.storage, { kind: "recovery", recoveryKey: "abc" }),
    ).rejects.toMatchObject({ code: "InvalidRecoveryKey" });
    const unrelated = await device("unrelated");
    const { recoveryKey: wrong } = await beginSetup(unrelated.deps(new FakeServer()), null);
    await expect(
      unlock(other.deps(server), storage.storage, { kind: "recovery", recoveryKey: wrong }),
    ).rejects.toMatchObject({ code: "KeySlotUnlock" });
  });

  it("再認証がないと 403 になり、同じ鍵のまま再認証してやり直せる", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const { keyId } = await beginSetup(deps, null);
    await expect(completeSetup(deps, keyId, null)).rejects.toBeInstanceOf(ReauthRequiredError);
    expect(server.ssss).toBeNull();
    await completeSetup(deps, keyId, TOKEN);
    expect(server.ssss?.default_key_id).toBe(keyId);
  });

  // 完了条件：各保存の直前と直後で止めて再起動しても、同じ鍵のまま最後まで終わる。
  const stops = [
    "POST /api/e2ee/secret_storage/keys",
    "POST /api/e2ee/cross_signing/keys",
    "POST /api/e2ee/cross_signing/signatures",
    "POST /api/e2ee/key_backup/versions",
  ];
  for (const path of stops) {
    for (const when of ["before", "after"] as const) {
      it(`${path} の${when === "before" ? "直前" : "直後"}で止めても最後まで終わる`, async () => {
        const server = new FakeServer();
        const first = await device("first");
        const { recoveryKey, keyId } = await beginSetup(first.deps(server), null);
        server.fail = { path, when };
        await expect(completeSetup(first.deps(server), keyId, TOKEN)).rejects.toThrow();

        if (!server.ssss) {
          // SSSS の保存より前で止まった。サーバーには何もないので、新しい鍵でやり直す。
          await first.start();
          expect(await loadE2eeStatus(first.deps(server))).toEqual({ kind: "notSetUp" });
          const retry = await beginSetup(first.deps(server), null);
          await completeSetup(first.deps(server), retry.keyId, TOKEN);
          expect((await loadE2eeStatus(first.deps(server))).kind).toBe("unlocked");
          return;
        }
        const masterBefore = server.master;
        // 再起動：メモリーの鍵は消える。SSSS をリカバリーキーで開け直して続ける。
        await first.start();
        const status = await loadE2eeStatus(first.deps(server));
        expect(status.kind).toBe("locked");
        if (status.kind !== "locked") throw new Error("unreachable");
        expect(status.reason).not.toBe("mismatch");
        const result = await unlock(first.deps(server), status.storage, {
          kind: "recovery",
          recoveryKey,
        });
        if (result.needsReauth) {
          await saveRemaining(first.deps(server), result.secretsRevision, TOKEN);
        }
        expect((await loadE2eeStatus(first.deps(server))).kind).toBe("unlocked");
        if (masterBefore) expect(server.master).toEqual(masterBefore);
        expect(server.versions).toHaveLength(1);
        expect(server.ssss.default_key_id).toBe(keyId);

        // 別の端末でも、失効や改ざんの表示を出さずに解錠できる。
        const other = await device("other");
        const otherStatus = await loadE2eeStatus(other.deps(server));
        if (otherStatus.kind !== "locked") throw new Error("unreachable");
        const otherResult = await unlock(other.deps(server), otherStatus.storage, {
          kind: "recovery",
          recoveryKey,
        });
        expect(otherResult.needsReauth).toBe(false);
      });
    }
  }

  it("解錠済みの端末は、起動時に止まった初回設定を見つける", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const { recoveryKey, keyId } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);
    // 版を失った（初回設定の途中を模す）。
    server.versions = [];
    await first.start();
    const status = await loadE2eeStatus(first.deps(server));
    expect(status).toMatchObject({ kind: "locked", reason: "resume" });
    if (status.kind !== "locked") throw new Error("unreachable");
    await unlock(first.deps(server), status.storage, { kind: "recovery", recoveryKey });
    expect(server.versions).toHaveLength(1);
  });

  it("2 つの端末が同時に始めると、後から保存した端末は先の端末の SSSS を解錠する", async () => {
    const server = new FakeServer();
    const a = await device("a");
    const b = await device("b");
    const setupA = await beginSetup(a.deps(server), null);
    const setupB = await beginSetup(b.deps(server), null);
    await completeSetup(a.deps(server), setupA.keyId, TOKEN);
    await expect(completeSetup(b.deps(server), setupB.keyId, TOKEN)).rejects.toBeInstanceOf(
      SetupConflictError,
    );
    const status = await loadE2eeStatus(b.deps(server));
    if (status.kind !== "locked") throw new Error("unreachable");
    await unlock(b.deps(server), status.storage, {
      kind: "recovery",
      recoveryKey: setupA.recoveryKey,
    });
    expect((await loadE2eeStatus(b.deps(server))).kind).toBe("unlocked");
  });

  it("サーバーの鍵が違えば、署名せずに失効か改ざんとして扱う", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const { recoveryKey, keyId } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);
    // 別の鍵（攻撃者の鍵）に差し替えられた。
    const attacker = new FakeServer();
    const evil = await device("evil");
    const evilSetup = await beginSetup(evil.deps(attacker), null);
    await completeSetup(evil.deps(attacker), evilSetup.keyId, TOKEN);
    server.master = attacker.master;
    server.selfSigning = attacker.selfSigning;

    const other = await device("other");
    const status = await loadE2eeStatus(other.deps(server));
    if (status.kind !== "locked") throw new Error("unreachable");
    const before = server.signatures.length;
    await expect(
      unlock(other.deps(server), status.storage, { kind: "recovery", recoveryKey }),
    ).rejects.toBeInstanceOf(ServerKeyMismatchError);
    expect(server.signatures).toHaveLength(before);
  });

  it("E2EE をリセットすると、新しい鍵と版に差し替わる", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const original = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), original.keyId, TOKEN);
    const oldMaster = server.master;

    // リカバリーキーをなくした別の端末からリセットする。
    const other = await device("other");
    const status = await loadE2eeStatus(other.deps(server));
    if (status.kind !== "locked") throw new Error("unreachable");
    const reset = await beginSetup(other.deps(server), status.storage);
    await completeSetup(other.deps(server), reset.keyId, TOKEN);
    expect(server.master).not.toEqual(oldMaster);
    expect(server.versions).toHaveLength(2);
    expect(server.ssss?.default_key_id).toBe(reset.keyId);
    expect((await loadE2eeStatus(other.deps(server))).kind).toBe("unlocked");
    // 元の端末は鍵が違うので、解錠し直しになる。
    expect(await loadE2eeStatus(first.deps(server))).toMatchObject({
      kind: "locked",
      reason: "mismatch",
    });
  });

  it("パスキー（PRF）のスロットを作り、別の端末でパスキーで解錠できる", async () => {
    const server = new FakeServer();
    const authenticator = virtualAuthenticator();
    const first = await device("first");
    const deps = first.deps(server, authenticator);
    const { keyId } = await beginSetup(deps, null);
    await addSetupPasskey(deps, "Test passkey");
    await completeSetup(deps, keyId, TOKEN);
    const slots = (server.ssss?.slots ?? []) as Json[];
    expect(slots.map((slot) => String(slot.kind)).sort((a, b) => a.localeCompare(b))).toEqual([
      "passkey",
      "recovery",
    ]);
    expect(slots.find((slot) => slot.kind === "passkey")?.label).toBe("Test passkey");

    const second = await device("second");
    const secondDeps = second.deps(server, authenticator);
    const status = await loadE2eeStatus(secondDeps);
    if (status.kind !== "locked") throw new Error("unreachable");
    await unlock(secondDeps, status.storage, { kind: "passkey" });
    expect((await loadE2eeStatus(secondDeps)).kind).toBe("unlocked");

    // 別のパスキーでは開けない（スロットがない）。
    const third = await device("third");
    const thirdDeps = third.deps(server, virtualAuthenticator("b3RoZXItY3JlZA"));
    const thirdStatus = await loadE2eeStatus(thirdDeps);
    if (thirdStatus.kind !== "locked") throw new Error("unreachable");
    await expect(unlock(thirdDeps, thirdStatus.storage, { kind: "passkey" })).rejects.toMatchObject(
      { code: "cancelled" },
    );
  });

  it("保存した鍵に、マスター鍵の秘密鍵と平文の秘密を入れない", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const { keyId } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);
    const keyRequest = server.requests.find((r) => r.path === "/api/e2ee/secret_storage/keys");
    expect(keyRequest?.reauth).toBe(true);
    const json = server.ssss as JsonObject;
    expect(JSON.stringify(json)).not.toContain("private");
  });

  it("SSSS を保存したあとに足したパスキーのスロットも、保存の再試行で反映する", async () => {
    const server = new FakeServer();
    const authenticator = virtualAuthenticator();
    const first = await device("first");
    const deps = first.deps(server, authenticator);
    const { keyId } = await beginSetup(deps, null);
    server.fail = { path: "POST /api/e2ee/cross_signing/keys", when: "before" };
    await expect(completeSetup(deps, keyId, TOKEN)).rejects.toThrow();
    expect(((server.ssss?.slots ?? []) as Json[]).map((slot) => slot.kind)).toEqual(["recovery"]);
    // 同じ画面のまま、パスキーを足して保存し直す。
    await addSetupPasskey(deps, null);
    await completeSetup(deps, keyId, TOKEN);
    const slots = (server.ssss?.slots ?? []) as Json[];
    expect(slots.map((slot) => slot.kind)).toEqual(["recovery", "passkey"]);
    expect(slots[1]).not.toHaveProperty("label");

    const second = await device("second");
    const secondDeps = second.deps(server, authenticator);
    const status = await loadE2eeStatus(secondDeps);
    if (status.kind !== "locked") throw new Error("unreachable");
    await unlock(secondDeps, status.storage, { kind: "passkey" });
    expect((await loadE2eeStatus(secondDeps)).kind).toBe("unlocked");
  });

  it("解錠済みの端末で、開け直した SSSS からラベルなしのパスキーを足せる", async () => {
    const server = new FakeServer();
    const authenticator = virtualAuthenticator();
    const first = await device("first");
    const { recoveryKey, keyId } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);
    // 読み込み直した：暗号ストアの鍵はあるが、SSSS 鍵はメモリーにない。
    await first.start();
    const deps = first.deps(server, authenticator);
    const status = await loadE2eeStatus(deps);
    if (status.kind !== "unlocked") throw new Error("unreachable");
    expect(await hasUnlockedSecrets(deps)).toBe(false);
    await unlock(deps, status.storage, { kind: "recovery", recoveryKey });
    expect(await hasUnlockedSecrets(deps)).toBe(true);
    await addPasskeySlot(deps, status.storage, null, TOKEN);
    expect(((server.ssss?.slots ?? []) as Json[]).map((slot) => slot.kind)).toEqual([
      "recovery",
      "passkey",
    ]);
  });

  it("起動時の確認は、解錠していない端末では何も取得せず、解錠済みなら署名し直す", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const { keyId } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);

    const other = await device("other");
    server.requests = [];
    await runStartupCheck(other.deps(server));
    expect(server.requests).toEqual([]);

    // 署名を失った（失効の途中で解錠した端末など）。起動時に署名し直す。
    server.signatures = [];
    await runStartupCheck(first.deps(server));
    expect(server.requests.map((r) => r.path)).toContain("/api/e2ee/cross_signing/signatures");
    expect(deviceSignatureCount(server)).toBe(1);
  });

  /** 別のユーザーの端末から、この端末に Megolm セッションの鍵を届ける（#359 の to-device の処理）。 */
  async function receiveRoomKey(target: Device): Promise<void> {
    const request = await target.worker.request({ op: "registrationRequest" });
    await target.worker.request({ op: "markRegistered" });
    let oneTimeKeys = request.one_time_keys;
    if (Object.keys(oneTimeKeys).length === 0) {
      // 登録で公開した鍵は使い切ったので、補充した鍵を使う。
      const upload = await target.worker.request({
        op: "prepareKeyUpload",
        oneTimeKeyCount: 0,
        hasUnusedFallbackKey: true,
      });
      oneTimeKeys = upload!.one_time_keys;
      await target.worker.request({ op: "markKeysPublished" });
    }
    const sender = new DeviceAccount("00000000-0000-4000-8000-00000000000b", crypto.randomUUID());
    const keys = request.device_keys;
    const recipient = verifyDeviceKeys(keys, USER, keys.device_id as string);
    const olm = sender.createOutboundOlmSession(recipient, Object.values(oneTimeKeys)[0]!);
    const outbound = new OutboundSession("c0000000-0000-4000-8000-000000000001", 1, Date.now());
    const content = sender.encryptRoomKey(olm, recipient, outbound.roomKey());
    const result = await target.worker.request({
      op: "processToDevice",
      rows: [
        {
          id: crypto.randomUUID(),
          sender_user_id: sender.userId,
          sender_device_id: sender.deviceId,
          type: "m.room.encrypted",
          content,
        },
      ],
      keysQuery: keysQueryOf({ [sender.userId]: { [sender.deviceId]: sender.deviceKeys() } }),
    });
    expect(result.roomKeys).toBe(1);
  }

  it("解錠した端末は、受け取ったセッションを鍵バックアップの今の版に送る", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    // 解錠していない端末は送らない（取得もしない）。
    await receiveRoomKey(first);
    server.requests = [];
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "done", uploaded: 0 });
    expect(server.requests).toEqual([]);

    // 初回設定を終えると、解錠前に受け取ったセッションも送る。
    const { keyId } = await beginSetup(deps, null);
    await completeSetup(deps, keyId, TOKEN);
    expect(server.backupSessions).toHaveLength(1);
    expect(await first.worker.request({ op: "pendingBackupCount" })).toBe(0);
    expect([...first.backend.inboundSessions.values()].every((s) => s.backedUp)).toBe(true);

    await receiveRoomKey(first);
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "done", uploaded: 1 });
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "done", uploaded: 0 });
    expect(server.backupSessions).toHaveLength(2);
  });

  it("新しい端末で解錠すると、鍵バックアップの今の版から過去のセッションをページごとに取り込む", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const { keyId, recoveryKey } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);
    for (let i = 0; i < 3; i += 1) await receiveRoomKey(first);
    expect(await uploadKeyBackup(first.deps(server))).toEqual({ kind: "done", uploaded: 3 });
    server.backupPageSize = 1;

    // 解錠を終えたら、取り込みを頼む（取り込みは待たずに返る）。
    const second = await device("second");
    let unlocked = 0;
    const deps = { ...second.deps(server), onUnlocked: () => (unlocked += 1) };
    const status = await loadE2eeStatus(deps);
    if (status.kind !== "locked") throw new Error("unreachable");
    await unlock(deps, status.storage, { kind: "recovery", recoveryKey });
    expect(unlocked).toBe(1);

    // 2 ページ目で通信が切れても、1 ページ目は保存してあり、次は 2 ページ目から読む。
    server.fail = { path: "GET /api/e2ee/key_backup/keys?version=1&after=1", when: "before" };
    const pages: number[] = [];
    await expect(importKeyBackup(deps, (n) => pages.push(n))).rejects.toThrow("network error");
    expect(pages).toEqual([1]);
    expect(second.backend.inboundSessions.size).toBe(1);
    server.requests = [];
    expect(await importKeyBackup(deps)).toEqual({ kind: "done", imported: 2, dropped: 0 });
    expect(server.requests.map((r) => r.path)).toContain(
      "/api/e2ee/key_backup/keys?version=1&after=1",
    );
    expect(server.requests.map((r) => r.path)).not.toContain("/api/e2ee/key_backup/keys?version=1");
    expect(second.backend.inboundSessions.size).toBe(3);

    // 取り込んだセッションは同じ版には上げ直さない。取り込み終えた版は読み直さない。
    server.requests = [];
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "done", uploaded: 0 });
    expect(await importKeyBackup(deps)).toEqual({ kind: "done", imported: 0, dropped: 0 });
    expect(server.requests.some((r) => r.path.startsWith("/api/e2ee/key_backup/keys"))).toBe(false);
    expect(server.backupSessions).toHaveLength(3);
  });

  it("起動時の鍵バックアップへの送信に失敗しても、鍵を使えるなら取り込みを始められる", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const { keyId } = await beginSetup(deps, null);
    await completeSetup(deps, keyId, TOKEN);
    await receiveRoomKey(first);
    server.fail = {
      path: `POST /api/e2ee/key_backup/keys?version=1&secrets_revision=${server.revision}`,
      when: "before",
    };
    expect(await runStartupCheck(deps)).toBe(true);
    expect(server.fail).toBeNull();
    expect(await first.worker.request({ op: "pendingBackupCount" })).toBe(1);
  });

  it("取り込みの途中で版が消えたら、やめて次の解錠か起動に任せる", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const { keyId, recoveryKey } = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), keyId, TOKEN);
    await receiveRoomKey(first);
    await uploadKeyBackup(first.deps(server));

    const second = await device("second");
    const deps = second.deps(server);
    const status = await loadE2eeStatus(deps);
    if (status.kind !== "locked") throw new Error("unreachable");
    await unlock(deps, status.storage, { kind: "recovery", recoveryKey });
    // 今の版を読んだあと、ページを読む前に別の端末が版を消した。
    const respond = server.client();
    const client = {
      ...respond,
      get: ((path: string, options?: RequestOptions) => {
        if (path.startsWith("/api/e2ee/key_backup/keys")) server.versions.at(-1)!.deleted = true;
        return respond.get(path, options);
      }) as ApiClient["get"],
    };
    expect(await importKeyBackup({ ...deps, client })).toEqual({
      kind: "gone",
      imported: 0,
      dropped: 0,
    });
  });

  it("今の版をマスター鍵で検証できないか、公開鍵が違えば送らない", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const { recoveryKey, keyId } = await beginSetup(deps, null);
    await completeSetup(deps, keyId, TOKEN);
    await receiveRoomKey(first);

    // 別の鍵（攻撃者の鍵）で作った版に差し替えられた。
    const attacker = new FakeServer();
    const evil = await device("evil");
    const evilSetup = await beginSetup(evil.deps(attacker), null);
    await completeSetup(evil.deps(attacker), evilSetup.keyId, TOKEN);
    const original = server.versions[0]!;
    server.versions = [{ ...attacker.versions[0]!, version: "2" }];
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "rejected", code: "MissingSignature" });
    expect(server.backupSessions).toHaveLength(0);

    // 今のマスター鍵で正しく署名されているが、公開鍵が SSSS のバックアップ鍵と違う版。
    // 署名を作るため、SSSS をリカバリーキーで開いて本物のマスター鍵を取り出す。
    const storage = SecretStorage.fromJson(USER, {
      ...server.ssss,
      secrets_revision: server.revision,
    } as unknown as JsonObject)!;
    const recovery = RecoveryKey.parse(recoveryKey);
    const secrets = OwnSecrets.fromUnlocked(storage.unlockWithRecoveryKey(recovery));
    const otherBackupKey = BackupDecryptionKey.generate();
    server.versions = [
      original,
      {
        ...original,
        version: "2",
        auth_data: otherBackupKey.authData(secrets.crossSigning.master),
      },
    ];
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "rejected", code: "BackupKeyMismatch" });
    expect(server.backupSessions).toHaveLength(0);
    expect(await first.worker.request({ op: "pendingBackupCount" })).toBe(1);

    // 元の版に戻れば送る（検証できなかった版は、見た版として数えない）。
    server.versions = [original];
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "done", uploaded: 1 });
    expect(server.backupSessions).toHaveLength(1);
  });

  it("秘密が書き換わったら（409）、送るのをやめて送信待ちに残す", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const { keyId } = await beginSetup(deps, null);
    await completeSetup(deps, keyId, TOKEN);
    await receiveRoomKey(first);
    server.revision += 1;
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "conflict", code: "E2EE_SSSS_CONFLICT" });
    expect(await first.worker.request({ op: "pendingBackupCount" })).toBe(1);
  });

  it("送信が重なっても、応答を受けた要求のセッションだけを送信済みにする", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const { keyId } = await beginSetup(deps, null);
    await completeSetup(deps, keyId, TOKEN);
    const currentVersion = (await deps.client.get("/api/e2ee/key_backup/versions/current")) as Json;

    // 要求 A（1 個）の応答を待つ間に、別の鍵を受け取って要求 B（2 個）を作る。
    await receiveRoomKey(first);
    const a = await first.worker.request({
      op: "prepareBackupUpload",
      currentVersion: currentVersion as JsonObject,
      limit: 1000,
    });
    await receiveRoomKey(first);
    const b = await first.worker.request({
      op: "prepareBackupUpload",
      currentVersion: currentVersion as JsonObject,
      limit: 1000,
    });
    if (a.kind !== "upload" || b.kind !== "upload") throw new Error("unreachable");
    expect(a.count).toBe(1);
    expect(b.count).toBe(2);
    // A だけが成功し、B は失敗した。B だけに入っていた鍵は送信待ちに残る。
    await first.worker.request({ op: "backupUploaded", uploadId: a.uploadId });
    await first.worker.request({ op: "backupUploadFailed", uploadId: b.uploadId });
    expect(await first.worker.request({ op: "pendingBackupCount" })).toBe(1);

    // 残った鍵は次の送信で送る（A の鍵を送り直すことはあるが、サーバーはよい方を残す）。
    expect((await uploadKeyBackup(deps)).kind).toBe("done");
    expect(await first.worker.request({ op: "pendingBackupCount" })).toBe(0);
    expect(new Set(server.backupSessions.map((row) => row.session_id)).size).toBe(2);
  });

  it("どの呼び出し元からの送信も、同じ端末では 1 つずつ行う", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const { keyId } = await beginSetup(deps, null);
    await completeSetup(deps, keyId, TOKEN);
    await receiveRoomKey(first);
    server.requests = [];
    const results = await Promise.all([uploadKeyBackup(deps), uploadKeyBackup(deps)]);
    expect(results.map((r) => (r.kind === "done" ? r.uploaded : -1)).sort((a, b) => a - b)).toEqual(
      [0, 1],
    );
    const posts = server.requests.filter((r) => r.path.startsWith("/api/e2ee/key_backup/keys"));
    expect(posts).toHaveLength(1);
    expect(server.backupSessions).toHaveLength(1);
  });

  it("再認証を待つ間に別の初回設定を始めたら、古い保存は新しい鍵を保存しない", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const a = await beginSetup(deps, null);
    const b = await beginSetup(deps, null);
    await expect(completeSetup(deps, a.keyId, TOKEN)).rejects.toMatchObject({
      code: "Superseded",
    });
    expect(server.ssss).toBeNull();
    await completeSetup(deps, b.keyId, TOKEN);
    expect(server.ssss?.default_key_id).toBe(b.keyId);
  });

  it("同時にリセットした別の端末の SSSS があれば、クロス署名鍵と版を差し替えない", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const original = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), original.keyId, TOKEN);

    // 端末 A のリセットは、SSSS を保存したあと、クロス署名鍵の保存の前で止まった。
    const a = await device("a");
    const aStatus = await loadE2eeStatus(a.deps(server));
    if (aStatus.kind !== "locked") throw new Error("unreachable");
    const resetA = await beginSetup(a.deps(server), aStatus.storage);
    server.fail = { path: "POST /api/e2ee/cross_signing/keys", when: "before" };
    await expect(completeSetup(a.deps(server), resetA.keyId, TOKEN)).rejects.toThrow();
    expect(server.ssss?.default_key_id).toBe(resetA.keyId);

    // その間に端末 B がリセットを終えた。
    const b = await device("b");
    const bStatus = await loadE2eeStatus(b.deps(server));
    if (bStatus.kind !== "locked") throw new Error("unreachable");
    const resetB = await beginSetup(b.deps(server), bStatus.storage);
    await completeSetup(b.deps(server), resetB.keyId, TOKEN);
    const masterB = server.master;

    // 端末 A が保存し直しても、B の鍵を A の鍵で差し替えない。
    await expect(completeSetup(a.deps(server), resetA.keyId, TOKEN)).rejects.toBeInstanceOf(
      SetupConflictError,
    );
    expect(server.master).toEqual(masterB);
    expect(server.ssss?.default_key_id).toBe(resetB.keyId);
    expect((await loadE2eeStatus(b.deps(server))).kind).toBe("unlocked");
  });

  it("版が変わったときに送信待ちへ戻せなくても、次の送信で戻して送り直す", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const deps = first.deps(server);
    const { keyId } = await beginSetup(deps, null);
    await receiveRoomKey(first);
    await completeSetup(deps, keyId, TOKEN);
    expect(server.backupSessions).toHaveLength(1);

    // 同じ鍵の新しい版に変わった。送信待ちに戻す書き込みが 1 回失敗する。
    server.versions.push({ ...server.versions[0]!, version: "2" });
    const reset = first.backend.resetBackedUp.bind(first.backend);
    let failures = 1;
    first.backend.resetBackedUp = async () => {
      if (failures-- > 0) throw new Error("storage down");
      return reset();
    };
    await expect(uploadKeyBackup(deps)).rejects.toThrow("storage down");
    expect(await uploadKeyBackup(deps)).toEqual({ kind: "done", uploaded: 1 });
    expect(server.backupSessions).toHaveLength(2);
  });

  it("読み直してから送るまでの間に別の端末がリセットしても、サーバーが前提の SSSS 鍵で拒む", async () => {
    const server = new FakeServer();
    const first = await device("first");
    const original = await beginSetup(first.deps(server), null);
    await completeSetup(first.deps(server), original.keyId, TOKEN);

    const a = await device("a");
    const aStatus = await loadE2eeStatus(a.deps(server));
    if (aStatus.kind !== "locked") throw new Error("unreachable");
    const resetA = await beginSetup(a.deps(server), aStatus.storage);

    // A が SSSS を保存し、続きのために SSSS と公開鍵を読み直す。SSSS は A のまま返り、公開鍵の
    // 取得を待つ間に、B がリセットを最後まで終える。
    const b = await device("b");
    let resetBKeyId = "";
    server.beforeKeysQuery = async () => {
      const bStatus = await loadE2eeStatus(b.deps(server));
      if (bStatus.kind !== "locked") throw new Error("unreachable");
      const resetB = await beginSetup(b.deps(server), bStatus.storage);
      await completeSetup(b.deps(server), resetB.keyId, TOKEN);
      resetBKeyId = resetB.keyId;
    };
    const masterBefore = () => server.master;
    await expect(completeSetup(a.deps(server), resetA.keyId, TOKEN)).rejects.toBeInstanceOf(
      SetupConflictError,
    );
    const masterB = masterBefore();
    expect(server.ssss?.default_key_id).toBe(resetBKeyId);
    // B のクロス署名鍵と版が残り、B は解錠済みのまま。
    expect(server.master).toEqual(masterB);
    expect((await loadE2eeStatus(b.deps(server))).kind).toBe("unlocked");
  });

  it("解錠のあと再認証を待つ間に別の端末が SSSS だけを作り直しても、続きを保存しない", async () => {
    const server = new FakeServer();
    // A の初回設定は、SSSS の保存のあと、クロス署名鍵の保存の前で止まった。
    const a = await device("a");
    const setupA = await beginSetup(a.deps(server), null);
    server.fail = { path: "POST /api/e2ee/cross_signing/keys", when: "before" };
    await expect(completeSetup(a.deps(server), setupA.keyId, TOKEN)).rejects.toThrow();

    // 端末 C が A のリカバリーキーで解錠する。続き（クロス署名鍵）には再認証が要る。
    const c = await device("c");
    const cStatus = await loadE2eeStatus(c.deps(server));
    if (cStatus.kind !== "locked") throw new Error("unreachable");
    expect(cStatus.reason).toBe("new");
    const result = await unlock(c.deps(server), cStatus.storage, {
      kind: "recovery",
      recoveryKey: setupA.recoveryKey,
    });
    expect(result).toMatchObject({ needsReauth: true, keyId: setupA.keyId });

    // その間に端末 B がリセットを始め、SSSS だけを保存して止まった。
    const b = await device("b");
    const bStatus = await loadE2eeStatus(b.deps(server));
    if (bStatus.kind !== "locked") throw new Error("unreachable");
    const resetB = await beginSetup(b.deps(server), bStatus.storage);
    server.fail = { path: "POST /api/e2ee/cross_signing/keys", when: "before" };
    await expect(completeSetup(b.deps(server), resetB.keyId, TOKEN)).rejects.toThrow();
    expect(server.ssss?.default_key_id).toBe(resetB.keyId);

    // C が再認証して続きを保存しようとしても、SSSS が B のものなので拒まれ、何も変わらない。
    await expect(
      saveRemaining(c.deps(server), result.secretsRevision, TOKEN, result.keyId),
    ).rejects.toBeInstanceOf(SetupConflictError);
    expect(server.master).toBeNull();
    expect(server.versions).toHaveLength(0);
  });
});
