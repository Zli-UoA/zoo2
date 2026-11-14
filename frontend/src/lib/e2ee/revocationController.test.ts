import { readFileSync } from "node:fs";

import {
  BackupEncryptionKey,
  DeviceAccount,
  RecoveryKey,
  SecretStorage,
  initSync,
  type JsonObject,
} from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it } from "vite-plus/test";

import { ApiError, type ApiClient, type RequestOptions } from "../api/client";
import type { CryptoWorker } from "./cryptoWorker";
import { CryptoOperations } from "./operations";
import type { CryptoOperation } from "./protocol";
import {
  RevocationChangedError,
  RevocationConflictError,
  RevocationPendingError,
  abandonRevocation,
  beginRevocation,
  continueRevocation,
  deleteSelectedDevices,
  loadDevices,
  newRevocationRecoveryKey,
  openSecretStorage,
  resumeRevocation,
  revocationStatus,
} from "./revocationController";
import { REAUTH_HEADER } from "./secretStorageApi";
import {
  ReauthRequiredError,
  beginSetup,
  completeSetup,
  type E2eeSetupDeps,
} from "./setupController";
import { MemoryCryptoBackend } from "./store/backend";
import type { WrappingKeyStore } from "./store/storageKey";

beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

const USER = "00000000-0000-4000-8000-00000000000a";
const CHANNEL = "00000000-0000-4000-8000-0000000000c1";
const TOKEN = "reauth-id-token";

type Json = Record<string, unknown>;

const keyOf = (key: Json | null) =>
  key ? Object.values(key.keys as Record<string, string>)[0] : null;

/**
 * docs/e2ee.md の API を、失効の手順に要る範囲だけ持つサーバー（自分のユーザーだけ）。
 * `fail` で、指定した要求の直前に通信を失敗させる。
 */
class FakeServer {
  ssss: Json | null = null;
  revision = 0;
  master: Json | null = null;
  selfSigning: Json | null = null;
  userSigning: Json | null = null;
  signatures: Json[] = [];
  versions: Json[] = [];
  sessions = new Map<string, Json[]>();
  devices: Json[] = [];
  pins: Json[] = [];
  fail: string | null = null;
  /** 指定した要求をサーバーに反映したあと、応答を返す前に `run` を待つ（割り込みを作るため）。 */
  after: { route: string; run: () => Promise<void> } | null = null;
  requests: { method: string; path: string; reauth: boolean }[] = [];

  client(): ApiClient {
    const handle = async (path: string, options: RequestOptions = {}) => {
      const method = options.method ?? "GET";
      const reauth = options.headers?.[REAUTH_HEADER] === TOKEN;
      this.requests.push({ method, path, reauth });
      const url = new URL(path, "http://zoo.test");
      if (this.fail === `${method} ${url.pathname}`) {
        this.fail = null;
        throw new TypeError("network error");
      }
      const result = this.respond(method, url, options.body as Json, reauth);
      if (this.after?.route === `${method} ${url.pathname}`) {
        const { run } = this.after;
        this.after = null;
        await run();
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

  /**
   * 実際のサーバーと同じく、前提にした既定の SSSS 鍵と `secrets_revision` を照合する（省略したものは
   * 照合しない）。
   */
  private checkExpectedDefaultKey(body: Json) {
    const current = (this.ssss?.default_key_id as string | undefined) ?? null;
    if (
      (body.expected_default_key_id !== undefined && body.expected_default_key_id !== current) ||
      (body.expected_secrets_revision !== undefined &&
        body.expected_secrets_revision !== (this.ssss ? this.revision : null))
    ) {
      this.error(409, "E2EE_SSSS_CONFLICT", { default_key_id: current });
    }
  }

  private requireReauth(reauth: boolean) {
    if (!reauth) this.error(403, "E2EE_REAUTH_REQUIRED");
  }

  private currentVersion(): Json | null {
    const current = this.versions.at(-1);
    return current && !current.deleted ? current : null;
  }

  private respond(method: string, url: URL, body: Json, reauth: boolean): unknown {
    const path = url.pathname;
    const route = `${method} ${path
      .replace(/\/[0-9a-f-]{36}(\/used)?$/, "/:id$1")
      .replace(/\/versions\/\d+$/, "/versions/:v")}`;
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
          slots: (body.slots as Json[]).map((slot) => ({ ...slot, key_id: keyId })),
        };
        this.revision += 1;
        return { secrets_revision: this.revision };
      }
      case "POST /api/e2ee/secret_storage/secrets": {
        if (!this.ssss || body.key_id !== this.ssss.default_key_id) {
          this.error(409, "E2EE_SSSS_CONFLICT");
        }
        if (body.expected_secrets_revision !== this.revision) this.error(409, "E2EE_SSSS_CONFLICT");
        const existing = this.ssss.secrets as Json;
        const names = Object.keys(body.secrets as Json);
        if (names.some((name) => name in existing)) this.requireReauth(reauth);
        for (const [name, secret] of Object.entries(body.secrets as Json)) {
          existing[name] = { ...(secret as Json), key_id: body.key_id };
        }
        this.revision += 1;
        return { secrets_revision: this.revision };
      }
      case "POST /api/e2ee/secret_storage/slots/:id/used":
        return {};
      case "GET /api/e2ee/devices":
        return this.devices;
      case "DELETE /api/e2ee/devices/:id": {
        const id = path.split("/").at(-1);
        this.devices = this.devices.filter((device) => device.device_id !== id);
        this.signatures = this.signatures.filter((row) => row.target_key_id !== id);
        return {};
      }
      case "GET /api/e2ee/tofu_pins":
        return { pins: this.pins, next: null };
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
        const current = this.currentVersion();
        if (!current) this.error(404, "NOT_FOUND");
        return current;
      }
      case "DELETE /api/e2ee/key_backup/versions/:v": {
        const version = this.versions.find((v) => v.version === path.split("/").at(-1));
        if (!version || version.deleted) this.error(404, "NOT_FOUND");
        if (version === this.currentVersion()) this.requireReauth(reauth);
        version.deleted = true;
        this.sessions.delete(version.version as string);
        return {};
      }
      case "GET /api/e2ee/key_backup/keys": {
        const version = url.searchParams.get("version") as string;
        const rows = this.sessions.get(version);
        if (!rows) this.error(404, "NOT_FOUND");
        return { sessions: rows, next: null };
      }
      case "POST /api/e2ee/key_backup/keys": {
        const version = url.searchParams.get("version");
        if (version !== this.currentVersion()?.version) {
          this.error(409, "E2EE_WRONG_BACKUP_VERSION");
        }
        if (Number(url.searchParams.get("secrets_revision")) !== this.revision) {
          this.error(409, "E2EE_SSSS_CONFLICT");
        }
        const rows = this.sessions.get(version as string) ?? [];
        rows.push(...(body.sessions as Json[]));
        this.sessions.set(version as string, rows);
        return { count: rows.length };
      }
      case "POST /api/e2ee/cross_signing/keys": {
        this.requireReauth(reauth);
        this.checkExpectedDefaultKey(body);
        const expected = body.expected_keys as Json;
        if (
          expected.master !== keyOf(this.master) ||
          expected.self_signing !== keyOf(this.selfSigning) ||
          expected.user_signing !== keyOf(this.userSigning)
        ) {
          this.error(409, "E2EE_CROSS_SIGNING_CONFLICT");
        }
        const replaced = [this.master, this.selfSigning, this.userSigning]
          .map(keyOf)
          .filter((key): key is string => key !== null);
        if (body.master_key) this.master = body.master_key as Json;
        if (body.self_signing_key) this.selfSigning = body.self_signing_key as Json;
        if (body.user_signing_key) this.userSigning = body.user_signing_key as Json;
        // 鍵を変えたら、古い鍵による署名を消す。
        const live = [this.master, this.selfSigning, this.userSigning].map(keyOf);
        const removed = replaced.filter((key) => !live.includes(key));
        this.signatures = this.signatures.filter(
          (row) => !removed.some((key) => row.signer_key_id === `ed25519:${key}`),
        );
        return {};
      }
      case "POST /api/e2ee/cross_signing/signatures":
        for (const signature of body.signatures as Json[]) {
          this.signatures.push({ signer_user_id: USER, ...signature });
        }
        return { count: (body.signatures as Json[]).length };
      case "POST /api/e2ee/tofu_pins":
        if (body.replace) this.requireReauth(reauth);
        return { pins: body.pins };
      case "POST /api/e2ee/key_backup/versions": {
        this.checkExpectedDefaultKey(body);
        const current = this.currentVersion();
        const authData = body.auth_data as Json;
        if (current && (current.auth_data as Json).public_key === authData.public_key) {
          return { version: current.version };
        }
        if (body.expected_current_version !== ((current?.version as string | undefined) ?? null)) {
          this.error(409, "E2EE_WRONG_BACKUP_VERSION");
        }
        if (this.versions.length > 0) this.requireReauth(reauth);
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

  /** 今の版に、送信端末の鍵が本物のセッションを `count` 件入れる。 */
  seedSessions(count: number): void {
    const current = this.currentVersion() as Json;
    const key = new BackupEncryptionKey((current.auth_data as Json).public_key as string);
    const sender = new DeviceAccount(USER, crypto.randomUUID());
    const identity = sender.identityKeys();
    const rows = Array.from({ length: count }, (_, i) => {
      const sessionId = `session-${i}`;
      return {
        channel_id: CHANNEL,
        sender_key: identity.curve25519,
        session_id: sessionId,
        first_message_index: 0,
        forwarded_count: 0,
        is_verified: true,
        session_data: key.encrypt({
          algorithm: "m.megolm.v1.aes-sha2",
          channel_id: CHANNEL,
          epoch: 0,
          session_id: sessionId,
          session_key: "exported",
          sender_key: identity.curve25519,
          sender_claimed_keys: { ed25519: identity.ed25519 },
          sender_user_id: USER,
          sender_device_id: crypto.randomUUID(),
        }),
      };
    });
    key.free();
    sender.free();
    this.sessions.set(current.version as string, rows as unknown as Json[]);
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
  private readonly backend = new MemoryCryptoBackend();
  private readonly keys = memoryKeys();
  worker!: CryptoWorker;
  deviceId!: string;

  constructor(private readonly namespace: string) {}

  async start(): Promise<void> {
    const operations = new CryptoOperations(
      { open: async () => this.backend, remove: async () => {} },
      this.keys,
    );
    this.worker = {
      request: (operation) => operations.run(operation as CryptoOperation) as never,
      terminate: () => {},
    };
    const opened = await this.worker.request({
      op: "open",
      namespace: this.namespace,
      userId: USER,
    });
    this.deviceId = opened.deviceId;
  }

  deps(server: FakeServer): E2eeSetupDeps {
    return { worker: this.worker, client: server.client(), userId: USER, rpId: null };
  }

  register(server: FakeServer, at = "2026-11-01T00:00:00Z"): void {
    server.devices.push({
      device_id: this.deviceId,
      display_name: this.namespace,
      device_keys: {},
      created_at: at,
      last_seen_at: at,
    });
  }
}

async function device(name: string): Promise<Device> {
  const created = new Device(name);
  await created.start();
  return created;
}

/** 初回設定を済ませたサーバーと端末。版 1 に 3 件のセッションがある。 */
async function setUp() {
  const server = new FakeServer();
  const first = await device("first");
  first.register(server);
  const deps = first.deps(server);
  const { recoveryKey, keyId } = await beginSetup(deps, null);
  await completeSetup(deps, keyId, TOKEN);
  server.seedSessions(3);
  return { server, first, deps, recoveryKey };
}

function liveVersions(server: FakeServer) {
  return server.versions.filter((v) => !v.deleted).map((v) => v.version);
}

function ssssPreviousKeys(server: FakeServer, recoveryKey: string): string[] {
  const storage = SecretStorage.fromJson(USER, {
    ...(server.ssss as JsonObject),
    secrets_revision: server.revision,
  });
  const recovery = RecoveryKey.parse(recoveryKey);
  const unlocked = storage!.unlockWithRecoveryKey(recovery);
  const list = unlocked.previousBackupKeys();
  const keys = Array.from({ length: list.length }, (_, i) => list.get(i)!.publicKey);
  list.free();
  unlocked.free();
  recovery.free();
  storage!.free();
  return keys;
}

describe("失効の手順", () => {
  it("端末だけの失効で、選んだ端末を消し、古い版のセッションを新しい版に移す", async () => {
    const { server, first, deps, recoveryKey } = await setUp();
    const lost = await device("lost");
    lost.register(server, "2026-11-02T00:00:00Z");
    const oldSelfSigning = keyOf(server.selfSigning);
    const oldBackup = (server.versions[0].auth_data as Json).public_key;

    const devices = await loadDevices(deps, first.deviceId);
    expect(devices.map((d) => [d.deviceId, d.current, d.signed])).toEqual([
      [first.deviceId, true, true],
      [lost.deviceId, false, false],
    ]);

    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await deleteSelectedDevices(deps, [lost.deviceId]);
    const begun = await beginRevocation(deps, "device_only", [lost.deviceId]);
    expect(begun).toMatchObject({ scope: "device_only", stage: "prepared" });
    // 始めただけでは、サーバーに何も保存しない。
    expect(keyOf(server.selfSigning)).toBe(oldSelfSigning);

    const stages: string[] = [];
    const result = await continueRevocation(deps, TOKEN, (summary) => stages.push(summary.stage));
    expect(stages).toEqual([
      "secret_storage_saved",
      "cross_signing_saved",
      "signatures_saved",
      "backup_created",
      "migrated",
    ]);
    expect(result.droppedSessions).toBe(0);
    expect(result.unsignedDevices).toEqual([]);
    expect(keyOf(server.selfSigning)).not.toBe(oldSelfSigning);
    expect(liveVersions(server)).toEqual(["2"]);
    expect(server.sessions.get("2")).toHaveLength(3);
    // 古い版の鍵は SSSS に退避した。
    expect(ssssPreviousKeys(server, recoveryKey)).toContain(oldBackup);
    expect(await revocationStatus(deps)).toBeNull();
    // 鍵を差し替える要求はすべて再認証付き。
    for (const request of server.requests.filter((r) =>
      [
        "/api/e2ee/secret_storage/secrets",
        "/api/e2ee/cross_signing/keys",
        "/api/e2ee/key_backup/versions",
      ].includes(r.path),
    )) {
      if (request.method === "POST") expect(request.reauth).toBe(true);
    }
  });

  it("保存の途中でページを閉じても、SSSS を開け直して続けられる", async () => {
    const { server, first, deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await beginRevocation(deps, "device_only", []);
    server.fail = "POST /api/e2ee/cross_signing/signatures";
    await expect(continueRevocation(deps, TOKEN)).rejects.toThrow(TypeError);

    // Worker を作り直す（メモリーの秘密を失い、暗号ストアの進み具合は残る）。
    await first.start();
    const restarted = first.deps(server);
    expect(await revocationStatus(restarted)).toMatchObject({ stage: "cross_signing_saved" });
    await openSecretStorage(restarted, { kind: "recovery", recoveryKey });
    expect(await resumeRevocation(restarted)).toMatchObject({
      kind: "resumed",
      summary: { stage: "cross_signing_saved" },
    });
    await continueRevocation(restarted, TOKEN);
    expect(liveVersions(server)).toEqual(["2"]);
    expect(await revocationStatus(restarted)).toBeNull();
  });

  it("再認証がなければ止まり、再認証してから続けられる", async () => {
    const { server, deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await beginRevocation(deps, "device_only", []);
    await expect(continueRevocation(deps, null)).rejects.toThrow(ReauthRequiredError);
    await continueRevocation(deps, TOKEN);
    expect(liveVersions(server)).toEqual(["2"]);
  });

  it("1〜5 の失効は新しいリカバリーキーを作り、SSSS の保存の前に止めたらやり直す", async () => {
    const { server, first, deps, recoveryKey } = await setUp();
    const oldMaster = keyOf(server.master);
    const oldKeyId = server.ssss?.default_key_id;
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    const lost = crypto.randomUUID();
    await beginRevocation(deps, "full", [lost]);
    // SSSS を保存する前に止めた。マスター鍵と SSSS 鍵は保存しないので続けられない。
    await first.start();
    const restarted = first.deps(server);
    await openSecretStorage(restarted, { kind: "recovery", recoveryKey });
    // やり直しでも、記録した範囲（1〜5）と削除した端末を引き継ぐ（端末だけに縮めない）。
    expect(await resumeRevocation(restarted)).toEqual({
      kind: "restart",
      scope: "full",
      deletedDevices: [lost],
    });
    expect(await revocationStatus(restarted)).toBeNull();

    await beginRevocation(restarted, "full", []);
    const newRecoveryKey = await newRevocationRecoveryKey(restarted);
    await continueRevocation(restarted, TOKEN);
    expect(keyOf(server.master)).not.toBe(oldMaster);
    expect(server.ssss?.default_key_id).not.toBe(oldKeyId);
    // 古いリカバリーキーでは開けず、新しいリカバリーキーで開ける。
    await expect(
      openSecretStorage(restarted, { kind: "recovery", recoveryKey }),
    ).rejects.toMatchObject({ code: "KeySlotUnlock" });
    await openSecretStorage(restarted, { kind: "recovery", recoveryKey: newRecoveryKey });
  });

  it("別の端末の失効が先にクロス署名鍵を保存したら、やり直しを求める", async () => {
    const { server, deps, recoveryKey } = await setUp();
    const other = await device("other");
    other.register(server);
    const otherDeps = other.deps(server);
    await openSecretStorage(otherDeps, { kind: "recovery", recoveryKey });

    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await beginRevocation(deps, "device_only", []);
    await beginRevocation(otherDeps, "device_only", []);
    await continueRevocation(otherDeps, TOKEN);
    await expect(continueRevocation(deps, TOKEN)).rejects.toBeInstanceOf(RevocationConflictError);
  });

  /**
   * A の 1〜5 の失効が SSSS を保存して `stop` の要求の前で止まったあと、B の 1〜5 の失効が SSSS だけを
   * 保存して止まる。A を続けても、B の SSSS に対応しない公開鍵や版を書かない。
   */
  async function interleaveSecretStorage(stop: string, scope: "full" | "device_only" = "full") {
    const { server, deps, recoveryKey } = await setUp();
    const other = await device("other");
    other.register(server);
    const otherDeps = other.deps(server);

    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await beginRevocation(deps, scope, []);
    const recoveryOfA = scope === "full" ? await newRevocationRecoveryKey(deps) : recoveryKey;
    server.fail = stop;
    await expect(continueRevocation(deps, TOKEN)).rejects.toThrow(TypeError);

    await openSecretStorage(otherDeps, { kind: "recovery", recoveryKey: recoveryOfA });
    await beginRevocation(otherDeps, scope, []);
    if (scope === "full") await newRevocationRecoveryKey(otherDeps);
    server.fail = "POST /api/e2ee/cross_signing/keys";
    await expect(continueRevocation(otherDeps, TOKEN)).rejects.toThrow(TypeError);
    const keyOfB = server.ssss?.default_key_id;
    const revisionOfB = server.revision;
    const selfSigning = keyOf(server.selfSigning);

    const master = keyOf(server.master);
    const versions = server.versions.length;
    const error = await continueRevocation(deps, TOKEN).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(RevocationConflictError);
    expect((error as RevocationConflictError).code).toBe("E2EE_SSSS_CONFLICT");
    expect(keyOf(server.master)).toBe(master);
    expect(server.versions).toHaveLength(versions);
    expect(server.ssss?.default_key_id).toBe(keyOfB);
    expect(server.revision).toBe(revisionOfB);
    expect(keyOf(server.selfSigning)).toBe(selfSigning);
  }

  it("SSSS の保存のあと別の端末が SSSS を作り直したら、クロス署名鍵を保存しない", async () => {
    await interleaveSecretStorage("POST /api/e2ee/cross_signing/keys");
  });

  it("版を作る前に別の端末が SSSS を作り直したら、版を作らない", async () => {
    await interleaveSecretStorage("POST /api/e2ee/key_backup/versions");
  });

  it("端末だけの失効でも、別の端末が SSSS の秘密だけを書き換えたら、公開鍵と版を保存しない", async () => {
    await interleaveSecretStorage("POST /api/e2ee/cross_signing/keys", "device_only");
    await interleaveSecretStorage("POST /api/e2ee/key_backup/versions", "device_only");
  });

  it("再認証のあいだに別の失効に置き換わったら、何も保存しない", async () => {
    const { server, deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    const first = await beginRevocation(deps, "device_only", []);
    // 画面を離れて、失効を捨ててから別の失効を始め直した。
    await abandonRevocation(deps);
    await beginRevocation(deps, "device_only", []);
    const before = server.requests.length;
    await expect(
      continueRevocation(deps, TOKEN, undefined, first.revocation_id),
    ).rejects.toBeInstanceOf(RevocationChangedError);
    expect(server.requests.slice(before).filter((r) => r.method !== "GET")).toEqual([]);
    // 今の失効は捨てていない。
    expect(await revocationStatus(deps)).not.toBeNull();
  });

  it("最後の一覧を読むあいだに別の失効に置き換わったら、今の失効を捨てさせない", async () => {
    const { server, deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await beginRevocation(deps, "device_only", []);
    let second: Awaited<ReturnType<typeof beginRevocation>> | null = null;
    server.after = {
      route: "GET /api/e2ee/devices",
      run: async () => {
        await abandonRevocation(deps);
        second = await beginRevocation(deps, "device_only", []);
      },
    };
    // 読み直しの不一致（`ServerKeyMismatchError`）ではなく、置き換わったことを返す。
    await expect(continueRevocation(deps, TOKEN)).rejects.toBeInstanceOf(RevocationChangedError);
    expect(await revocationStatus(deps)).toMatchObject({ revocation_id: second!.revocation_id });
  });

  it("再開しようとした失効が別の失効に置き換わったら、続けない", async () => {
    const { server, deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await beginRevocation(deps, "device_only", []);
    let second: Awaited<ReturnType<typeof beginRevocation>> | null = null;
    // 記録を読んでから続けるまでのあいだに、別の画面で A を捨てて B を始める。
    server.after = {
      route: "GET /api/e2ee/secret_storage",
      run: async () => {
        await abandonRevocation(deps);
        second = await beginRevocation(deps, "device_only", []);
      },
    };
    await expect(resumeRevocation(deps)).rejects.toBeInstanceOf(RevocationChangedError);
    expect(await revocationStatus(deps)).toMatchObject({ revocation_id: second!.revocation_id });
  });

  it("捨てる失効を指定したら、その失効の記録だけを捨てる", async () => {
    const { deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    const first = await beginRevocation(deps, "device_only", []);
    await expect(abandonRevocation(deps, "another")).rejects.toBeInstanceOf(RevocationChangedError);
    expect(await revocationStatus(deps)).toMatchObject({ revocation_id: first.revocation_id });
    await abandonRevocation(deps, first.revocation_id);
    expect(await revocationStatus(deps)).toBeNull();
  });

  it("終わっていない失効があれば、新しい失効を始めない", async () => {
    const { server, deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    const first = await beginRevocation(deps, "device_only", []);
    const before = server.requests.length;
    await expect(beginRevocation(deps, "full", [])).rejects.toBeInstanceOf(RevocationPendingError);
    // 前の失効はそのまま残り、サーバーには何も書かない（読むだけ）。
    expect(await revocationStatus(deps)).toMatchObject({ revocation_id: first.revocation_id });
    expect(
      server.requests
        .slice(before)
        .filter((r) => r.method !== "GET" && r.path !== "/api/e2ee/keys/query"),
    ).toEqual([]);
  });

  it("保存の応答を待つあいだに別の失効に置き換わったら、新しい失効の段を進めない", async () => {
    const { server, deps, recoveryKey } = await setUp();
    await openSecretStorage(deps, { kind: "recovery", recoveryKey });
    await beginRevocation(deps, "device_only", []);
    let second: Awaited<ReturnType<typeof beginRevocation>> | null = null;
    // A の SSSS の保存がサーバーに届いたあと、応答が返る前に、画面を離れて A を捨て、B を始める。
    server.after = {
      route: "POST /api/e2ee/secret_storage/secrets",
      run: async () => {
        await abandonRevocation(deps);
        second = await beginRevocation(deps, "device_only", []);
      },
    };
    await expect(continueRevocation(deps, TOKEN)).rejects.toBeInstanceOf(RevocationChangedError);
    // B は始めたときの段のまま（A の応答で SSSS の保存を飛ばさない）。
    expect(await revocationStatus(deps)).toMatchObject({
      revocation_id: second!.revocation_id,
      stage: "prepared",
    });
  });
});
