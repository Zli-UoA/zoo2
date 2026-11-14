import { readFileSync } from "node:fs";

import { OwnSecrets, initSync, type JsonObject } from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it } from "vite-plus/test";

import { ApiError, type ApiClient, type RequestOptions } from "../api/client";
import type { CryptoWorker } from "./cryptoWorker";
import { CryptoOperations, toCryptoFailure } from "./operations";
import type { CryptoOperation, ToDeviceRow } from "./protocol";
import { SasController, type SasEntry } from "./sasController";
import { REAUTH_HEADER } from "./secretStorageApi";
import { MemoryCryptoBackend } from "./store/backend";
import type { CryptoStore } from "./store/cryptoStore";
import type { WrappingKeyStore } from "./store/storageKey";

beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

const ALICE = "00000000-0000-4000-8000-00000000000a";
const BOB = "00000000-0000-4000-8000-00000000000b";
const TOKEN = "reauth-id-token";

type Json = Record<string, unknown>;

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

/** 1 人のユーザーの 1 つの端末（Worker の代わりに CryptoOperations を直接呼ぶ）。 */
interface Party {
  userId: string;
  deviceId: string;
  store: CryptoStore;
  worker: CryptoWorker;
  sas: SasController;
  /** サーバーに置いた公開鍵。 */
  master: JsonObject;
  selfSigning: JsonObject;
  userSigning: JsonObject;
  deviceKeys: JsonObject;
  oneTimeKeys: [string, JsonObject][];
}

/**
 * docs/e2ee.md の API のうち、SAS の検証に要る範囲（keys/query、keys/claim、to-device、ピン、署名）を
 * 持つサーバー。
 */
class FakeServer {
  parties = new Map<string, Party>();
  queue: (ToDeviceRow & { recipient_user_id: string; recipient_device_id: string })[] = [];
  signatures: Json[] = [];
  pins = new Map<string, Map<string, Json>>();
  nextId = 1;
  requests: { user: string; route: string; reauth: boolean }[] = [];
  /** この回数だけ、`keys/query` を 503 にする。 */
  failKeysQuery = 0;
  /** この端末あての to-device を、キューに入れずに `undelivered` で返す。 */
  undeliver = new Set<string>();

  client(userId: string): ApiClient {
    const handle = async (path: string, options: RequestOptions = {}) => {
      const method = options.method ?? "GET";
      const reauth = options.headers?.[REAUTH_HEADER] === TOKEN;
      const route = `${method} ${new URL(path, "http://zoo.test").pathname}`;
      this.requests.push({ user: userId, route, reauth });
      return this.respond(userId, route, (options.body ?? {}) as Json, reauth);
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

  private respond(caller: string, route: string, body: Json, reauth: boolean): unknown {
    switch (route) {
      case "POST /api/e2ee/keys/query":
        if (this.failKeysQuery > 0) {
          this.failKeysQuery -= 1;
          throw new ApiError(503, "UNAVAILABLE", { error: "unavailable" });
        }
        return this.keysQuery(caller, body.user_ids as string[]);
      case "POST /api/e2ee/keys/claim": {
        const out: Json = {};
        for (const [user, devices] of Object.entries(
          body.one_time_keys as Record<string, Record<string, string>>,
        )) {
          for (const device of Object.keys(devices)) {
            const party = this.parties.get(user);
            const key = party?.deviceId === device ? party.oneTimeKeys.shift() : undefined;
            if (key) ((out[user] ??= {}) as Json)[device] = { [key[0]]: key[1] };
          }
        }
        return { one_time_keys: out };
      }
      case "POST /api/e2ee/to_device": {
        const undelivered: Json[] = [];
        for (const [user, devices] of Object.entries(
          body.messages as Record<string, Record<string, JsonObject>>,
        )) {
          for (const [device, content] of Object.entries(devices)) {
            if (this.undeliver.has(device)) {
              undelivered.push({ user_id: user, device_id: device });
              continue;
            }
            this.queue.push({
              id: String(this.nextId++),
              recipient_user_id: user,
              recipient_device_id: device,
              sender_user_id: caller,
              sender_device_id: body.device_id as string,
              type: body.type as string,
              content,
            });
          }
        }
        return { undelivered };
      }
      case "POST /api/e2ee/tofu_pins": {
        if (body.replace && !reauth) this.reauthRequired();
        const mine = this.pins.get(caller) ?? new Map<string, Json>();
        this.pins.set(caller, mine);
        for (const pin of body.pins as Json[]) {
          mine.set(pin.target_user_id as string, {
            type: "zoo.tofu_pin",
            user_id: caller,
            ...pin,
          });
        }
        return { pins: [...mine.values()] };
      }
      case "POST /api/e2ee/cross_signing/signatures": {
        if (!reauth) this.reauthRequired();
        for (const signature of body.signatures as Json[]) {
          this.signatures.push({ signer_user_id: caller, ...signature });
        }
        return {};
      }
      default:
        throw new ApiError(404, "NOT_FOUND", { error: route });
    }
  }

  private reauthRequired(): never {
    throw new ApiError(403, "E2EE_REAUTH_REQUIRED", {
      error: "E2EE_REAUTH_REQUIRED",
      code: "E2EE_REAUTH_REQUIRED",
    });
  }

  keysQuery(caller: string, userIds: string[]): JsonObject {
    const out = {
      device_keys: {} as Json,
      master_keys: {} as Json,
      self_signing_keys: {} as Json,
      user_signing_keys: {} as Json,
      tofu_pins: {} as Json,
      signatures: [] as Json[],
    };
    for (const user of new Set([...userIds, caller])) {
      const party = this.parties.get(user);
      if (!party) continue;
      out.device_keys[user] = { [party.deviceId]: party.deviceKeys };
      out.master_keys[user] = party.master;
      out.self_signing_keys[user] = party.selfSigning;
      if (user === caller) out.user_signing_keys[user] = party.userSigning;
      const pin = this.pins.get(caller)?.get(user);
      if (pin) out.tofu_pins[user] = pin;
    }
    out.signatures = this.signatures;
    return out as unknown as JsonObject;
  }

  /** 届いている to-device を、宛先の端末で処理し、検証のメッセージをコントローラーに渡す。 */
  async pump(): Promise<void> {
    for (let round = 0; round < 50 && this.queue.length > 0; round += 1) {
      const rows = this.queue.splice(0);
      for (const party of this.parties.values()) {
        const mine = rows.filter(
          (row) =>
            row.recipient_user_id === party.userId && row.recipient_device_id === party.deviceId,
        );
        if (mine.length === 0) continue;
        const senders = [...new Set(mine.map((row) => row.sender_user_id))];
        const deviceKeys = this.keysQuery(party.userId, senders).device_keys as Record<
          string,
          Record<string, JsonObject>
        >;
        const result = await party.worker.request({
          op: "processToDevice",
          rows: mine,
          deviceKeys,
        });
        expect(result.dropped).toBe(0);
        await party.sas.handleMessages(result.verifications);
      }
    }
    expect(this.queue).toEqual([]);
  }
}

async function join(server: FakeServer, userId: string, unlocked = true): Promise<Party> {
  const operations = new CryptoOperations(
    {
      open: async () => new MemoryCryptoBackend(),
      remove: async () => {},
    },
    memoryKeys(),
  );
  const worker: CryptoWorker = {
    request: async (operation) => {
      try {
        return (await operations.run(operation as CryptoOperation)) as never;
      } catch (error) {
        throw Object.assign(new Error(), toCryptoFailure(error));
      }
    },
    terminate: () => {},
  };
  const opened = await worker.request({ op: "open", namespace: userId, userId });
  const store = (operations as unknown as { store: CryptoStore }).store;
  const registration = store.registrationRequest();
  const secrets = OwnSecrets.generate(userId);
  const cs = secrets.crossSigning;
  const upload = cs.uploadJson() as Json;
  const selfSigning = cs.selfSigning;
  const signed = selfSigning.signDevice(registration.device_keys);
  selfSigning.free();
  cs.free();
  const deviceKeys = {
    ...registration.device_keys,
    signatures: {
      ...(registration.device_keys.signatures as Json),
      [userId]: {
        ...(registration.device_keys.signatures as Record<string, Json>)[userId],
        [signed.signer_key_id]: signed.signature,
      },
    },
  } as JsonObject;
  if (unlocked) await store.saveOwnKeys(secrets, 1);
  else secrets.free();
  await store.markRegistered();
  const party: Party = {
    userId,
    deviceId: opened.deviceId,
    store,
    worker,
    sas: new SasController({
      worker,
      client: server.client(userId),
      userId,
      deviceId: opened.deviceId,
      expireIntervalMs: 0,
    }),
    master: upload.master_key as JsonObject,
    selfSigning: upload.self_signing_key as JsonObject,
    userSigning: upload.user_signing_key as JsonObject,
    deviceKeys,
    oneTimeKeys: Object.entries(registration.one_time_keys),
  };
  server.parties.set(userId, party);
  return party;
}

async function until(check: () => boolean): Promise<void> {
  for (let i = 0; i < 200; i += 1) {
    if (check()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error("時間内に条件を満たしませんでした");
}

function only(party: Party): SasEntry {
  const entries = party.sas.getSnapshot();
  expect(entries).toHaveLength(1);
  return entries[0]!;
}

/** Alice が Bob に検証を求め、Bob が受け入れて、双方が絵文字を見る段階まで進める。 */
async function startVerification(server: FakeServer, alice: Party, bob: Party) {
  const txnId = await alice.sas.request(BOB);
  expect(only(alice).view.phase).toBe("requested");
  await server.pump();
  expect(only(bob).view).toMatchObject({
    transaction_id: txnId,
    their_user_id: ALICE,
    phase: "incoming",
    we_requested: false,
  });
  await bob.sas.accept(txnId);
  await server.pump();
  expect(only(alice).view.phase).toBe("compare");
  expect(only(bob).view.phase).toBe("compare");
  return txnId;
}

describe("絵文字 SAS 検証", () => {
  it("2 人が互いに検証し、相手のマスター鍵への署名とピンを保存して、検証済みになる", async () => {
    const server = new FakeServer();
    const alice = await join(server, ALICE);
    const bob = await join(server, BOB);
    const txnId = await startVerification(server, alice, bob);
    const emoji = only(alice).view.emoji;
    expect(emoji).toHaveLength(7);
    expect(only(bob).view.emoji).toEqual(emoji);

    await alice.sas.confirm(txnId, Promise.resolve(TOKEN));
    await server.pump();
    expect(only(alice).view.phase).toBe("confirmed");
    await bob.sas.confirm(txnId, Promise.resolve(TOKEN));
    await server.pump();
    expect(only(alice).view.phase).toBe("done");
    expect(only(bob).view.phase).toBe("done");
    await until(() => only(alice).save === "saved" && only(bob).save === "saved");

    // 相手のマスター鍵への署名と、replace: true のピンを、再認証つきで保存した。
    expect(server.signatures.map((s) => [s.signer_user_id, s.target_user_id])).toEqual(
      expect.arrayContaining([
        [ALICE, BOB],
        [BOB, ALICE],
      ]),
    );
    expect(server.pins.get(ALICE)?.get(BOB)?.master_key).toBe(
      Object.values(bob.master.keys as Record<string, string>)[0],
    );
    expect(
      server.requests.filter((r) => r.route === "POST /api/e2ee/tofu_pins").every((r) => r.reauth),
    ).toBe(true);

    // 配布先の判定で「検証済み」（ユーザー署名鍵の署名）になる。
    const evaluation = await alice.worker.request({
      op: "trustEvaluate",
      keysQuery: server.keysQuery(ALICE, [BOB]),
      userIds: [BOB],
    });
    expect(evaluation.users[BOB]?.state).toEqual({
      state: "trusted",
      basis: "user_signing_signature",
    });
    // 記録の登録待ちは、ピンと署名を保存できてから外れている。
    expect(alice.store.trustStore.record(BOB)).toMatchObject({ pin_pending: false });

    await alice.sas.dismiss(txnId);
    expect(alice.sas.getSnapshot()).toEqual([]);
  });

  it("絵文字が一致しなければ、双方で取りやめ、署名しない", async () => {
    const server = new FakeServer();
    const alice = await join(server, ALICE);
    const bob = await join(server, BOB);
    const txnId = await startVerification(server, alice, bob);
    await bob.sas.mismatch(txnId);
    await server.pump();
    expect(only(bob).view.cancel).toEqual({ code: "m.mismatched_sas", by_us: true });
    expect(only(alice).view).toMatchObject({
      phase: "cancelled",
      cancel: { code: "m.mismatched_sas", by_us: false },
    });
    expect(server.signatures).toEqual([]);
    expect(alice.store.trustStore.record(BOB)).toBeUndefined();
  });

  it("再認証できなければ保存せず、再認証し直すと保存する", async () => {
    const server = new FakeServer();
    const alice = await join(server, ALICE);
    const bob = await join(server, BOB);
    const txnId = await startVerification(server, alice, bob);
    await alice.sas.confirm(txnId, Promise.reject(new Error("popup blocked")));
    await bob.sas.confirm(txnId, Promise.resolve(TOKEN));
    await server.pump();
    await until(() => only(alice).save === "reauth");
    expect(server.signatures.filter((s) => s.signer_user_id === ALICE)).toEqual([]);
    // 署名する前なので、記録も変えていない。
    expect(alice.store.trustStore.record(BOB)).toBeUndefined();

    await alice.sas.retrySave(txnId, Promise.resolve(TOKEN));
    expect(only(alice).save).toBe("saved");
    expect(server.signatures.filter((s) => s.signer_user_id === ALICE)).toHaveLength(1);
  });

  it("サーバーに保存できなければ、記録は登録待ちのまま", async () => {
    const server = new FakeServer();
    const alice = await join(server, ALICE);
    const bob = await join(server, BOB);
    const txnId = await startVerification(server, alice, bob);
    // 再認証の ID トークンがサーバーで認められない（古い）。
    await alice.sas.confirm(txnId, Promise.resolve("stale-token"));
    await bob.sas.confirm(txnId, Promise.resolve(TOKEN));
    await server.pump();
    await until(() => only(alice).save === "reauth");
    expect(alice.store.trustStore.record(BOB)).toMatchObject({ pin_pending: true });
  });

  it("解錠していない端末は、要求を受け入れられない", async () => {
    const server = new FakeServer();
    const alice = await join(server, ALICE);
    await join(server, BOB, false);
    await expect(alice.sas.request(BOB)).resolves.toBeTypeOf("string");
    await server.pump();
    // 解錠していない端末は自分のマスター鍵を確かめていないので、要求を受け付けない（何も出さない）。
    expect(server.parties.get(BOB)!.sas.getSnapshot()).toEqual([]);
  });

  it("鍵が要らないメッセージでは keys/query を取らず、取れないときは捨てずに試し直す", async () => {
    const server = new FakeServer();
    const alice = await join(server, ALICE);
    const bob = await join(server, BOB);
    const txnId = await alice.sas.request(BOB);
    await server.pump();
    // Bob の ready を Alice が処理するときの keys/query が失敗する。
    await bob.sas.accept(txnId);
    server.failKeysQuery = 1;
    await server.pump();
    expect(only(alice).view.phase).toBe("requested");
    expect(only(alice).error).toBe("receiveFailed");
    // 期限の確認で試し直すと進む。
    await alice.sas.expire();
    await server.pump();
    expect(only(alice).view.phase).toBe("compare");
    expect(only(alice).error).toBeNull();

    // key、mac、done の処理では keys/query を取らないので、失敗していても検証は終わる。
    server.failKeysQuery = 1_000;
    await alice.sas.confirm(txnId, Promise.resolve(TOKEN));
    await bob.sas.confirm(txnId, Promise.resolve(TOKEN));
    await server.pump();
    expect(only(alice).view.phase).toBe("done");
    expect(only(bob).view.phase).toBe("done");
  });

  it("相手の端末に届けられなければ、送れなかったと出す", async () => {
    const server = new FakeServer();
    const alice = await join(server, ALICE);
    const bob = await join(server, BOB);
    server.undeliver.add(bob.deviceId);
    await alice.sas.request(BOB);
    expect(only(alice).error).toBe("sendFailed");
    expect(server.queue).toEqual([]);
  });

  it("10 分たつと取りやめ、相手にも知らせる", async () => {
    const server = new FakeServer();
    let now = Date.now();
    const alice = await join(server, ALICE);
    const bob = await join(server, BOB);
    const clocked = new SasController({
      worker: alice.worker,
      client: server.client(ALICE),
      userId: ALICE,
      deviceId: alice.deviceId,
      now: () => now,
      expireIntervalMs: 0,
    });
    alice.sas = clocked;
    await clocked.request(BOB);
    await server.pump();
    expect(only(bob).view.phase).toBe("incoming");
    now += 10 * 60 * 1000;
    await clocked.expire();
    expect(only(alice).view.cancel).toEqual({ code: "m.timeout", by_us: true });
    await server.pump();
    expect(only(bob).view.cancel).toEqual({ code: "m.timeout", by_us: false });
  });
});
