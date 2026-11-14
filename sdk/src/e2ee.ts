/**
 * Bot E2EE device for Zoo's encrypted channels (docs/e2ee.md, "Bot の端末", issue #366).
 *
 * A bot joins an encrypted channel as an E2EE device, like a human client:
 *
 * - It owns a device (Olm identity keys and one-time keys) and its own cross-signing keys, and signs
 *   its device with its self-signing key. Members' clients trust the bot's master key on first use
 *   (TOFU) and then share the channel's Megolm session keys with the bot's signed device.
 * - It receives Megolm session keys as Olm to-device messages (`/api/apps/v1/e2ee/to_device`) and
 *   decrypts `message.created` / `message.updated` events whose `data.encrypted` is `true`.
 * - It encrypts its own messages with its own Megolm session, which it shares with the verified
 *   devices of the channel's snapshot (`/api/apps/v1/e2ee/channels/{id}/recipients`).
 *
 * The App must request the `e2ee:device` scope, and a channel member must explicitly allow the bot
 * when adding it to an encrypted channel. Nothing here is sent to Zoo in plaintext.
 *
 * Key storage: every secret (device account, Olm and Megolm sessions, self-signing and user-signing
 * keys, trust records) is pickled with a 32-byte storage key that the operator provides (for example
 * `ZOO_E2EE_STORAGE_KEY`) and written to an {@link E2eeStore} (a file by default). The master key's
 * private key is discarded after setup, so losing the store means setting the device up again, which
 * members' clients see as a changed master key (they must re-verify before sharing keys again).
 *
 * The Rust core (`crates/zoo-crypto`) is loaded through its wasm build (`crates/zoo-crypto-wasm`,
 * built with `crates/zoo-crypto-wasm/build.sh`); see {@link loadZooCrypto}.
 */
import { randomUUID } from "node:crypto";
import { open, readFile, rename } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { ZooError, type AppEvent, type ZooApp } from "./index.ts";

/** The wasm module produced by `crates/zoo-crypto-wasm/build.sh`. */
export type ZooCrypto = typeof import("../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm.js");
type Wasm = ZooCrypto;
type DeviceAccount = Wasm["DeviceAccount"]["prototype"];
type OlmSessions = Wasm["OlmSessions"]["prototype"];
type InboundSessionStore = Wasm["InboundSessionStore"]["prototype"];
type OutboundSession = Wasm["OutboundSession"]["prototype"];
type OwnKeys = Wasm["OwnKeys"]["prototype"];
type TrustStore = Wasm["TrustStore"]["prototype"];
type DeviceKeys = Wasm["DeviceKeys"]["prototype"];
type JsonObject = import("../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm.js").JsonObject;

/** Keep this many one-time keys on the server (docs/e2ee.md, the same value as the web client). */
const ONE_TIME_KEY_TARGET = 50;
/** Give up a send after this many `E2EE_STALE_EPOCH` conflicts in a row (as the web client does). */
const STALE_EPOCH_ATTEMPTS = 3;
/** Server limits (docs/e2ee.md). */
const KEYS_CLAIM_BATCH = 1000;
const TO_DEVICE_BATCH = 100;
const TO_DEVICE_PAGE = 100;
/** How many processed to-device ids to remember (to ACK again without decrypting twice). */
const PROCESSED_TO_DEVICE_MEMORY = 1000;
/** Waits before reading a channel's snapshot again while verifying a received session key. */
const EVALUATE_RETRY_MS = [200, 1000, 3000];
/** Waits before retrying a message whose room key has not arrived yet. */
const KEY_WAIT_MS = [200, 500, 1000, 2000];

/**
 * Load the zoo-crypto wasm package in Node. `pkgDir` defaults to `ZOO_CRYPTO_WASM_DIR`, then to
 * `crates/zoo-crypto-wasm/pkg` of this repository.
 */
export async function loadZooCrypto(pkgDir = process.env.ZOO_CRYPTO_WASM_DIR): Promise<ZooCrypto> {
  const dir =
    pkgDir ?? fileURLToPath(new URL("../../crates/zoo-crypto-wasm/pkg/", import.meta.url));
  const module = (await import(pathToFileURL(join(dir, "zoo_crypto_wasm.js")).href)) as ZooCrypto;
  module.initSync({ module: await readFile(join(dir, "zoo_crypto_wasm_bg.wasm")) });
  return module;
}

/** Where the bot keeps its pickled (encrypted) crypto state. */
export interface E2eeStore {
  load(): Promise<string | null>;
  save(data: string): Promise<void>;
}

/** A file store: written atomically (temporary file and rename) and readable only by its owner. */
export function fileStore(path: string): E2eeStore {
  return {
    async load() {
      try {
        return await readFile(path, "utf8");
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
        throw error;
      }
    },
    async save(data) {
      const temporary = `${path}.${process.pid}.tmp`;
      const file = await open(temporary, "w", 0o600);
      try {
        await file.writeFile(data);
        // Flush before the rename, so a power loss never leaves an empty store (which would mean a
        // new master key and re-verification by every member).
        await file.sync();
      } finally {
        await file.close();
      }
      await rename(temporary, path);
    },
  };
}

/** An in-memory store (tests). */
export function memoryStore(initial: string | null = null): E2eeStore & { data: string | null } {
  const store = {
    data: initial,
    async load() {
      return store.data;
    },
    async save(data: string) {
      store.data = data;
    },
  };
  return store;
}

export interface E2eeOptions {
  app: ZooApp;
  crypto: ZooCrypto;
  store: E2eeStore;
  /** 32 random bytes that encrypt everything in the store. Keep it out of the store and the logs. */
  storageKey: Uint8Array;
  /** The device name shown to the bot's owner (not to other users). */
  deviceName?: string;
  /** For tests. */
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
}

export interface Mentions {
  user_ids: string[];
  channel: boolean;
  here: boolean;
}

/** A decrypted message of an encrypted channel. */
export interface DecryptedMessage {
  id: string;
  channelId: string;
  parentId: string | null;
  userId: string;
  body: string;
  kind: "message" | "emote";
  mentions: Mentions;
  revision: number;
  /**
   * Whether the sending device was verified (signed by the sender's trusted master key) when the bot
   * received its session key. Bots should not act on commands from unverified senders.
   */
  senderVerified: boolean;
}

/** A send that kept failing because the channel's membership changed (`E2EE_STALE_EPOCH`). */
export class E2eeSendError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "E2eeSendError";
  }
}

interface StoredState {
  version: 1;
  user_id: string;
  device_id: string;
  account: string;
  own_keys: string;
  trust: string;
  olm_sessions: { session_id: string; their_identity_key: string; pickle: string }[];
  /** Outbound Olm sessions by the recipient's identity key (kept apart from inbound ones). */
  outbound_olm: Record<string, string>;
  /** Inbound Megolm sessions by `channel_id|sender_key|session_id`. */
  inbound: Record<string, string>;
  processed_to_device: string[];
}

interface Outbound {
  session: OutboundSession;
  /** `user_id|device_id` of the devices that received the session key. */
  shared: Set<string>;
}

interface Evaluation {
  epoch: number;
  pages: JsonObject[];
  devices: DeviceKeys[];
}

interface ToDeviceRow {
  id: string;
  sender_user_id: string;
  sender_device_id: string;
  type: string;
  content: JsonObject;
}

const inboundKey = (ref: { channel_id: string; sender_key: string; session_id: string }) =>
  `${ref.channel_id}|${ref.sender_key}|${ref.session_id}`;

/** The bot's E2EE device. Create it with {@link ZooE2ee.open}. */
export class ZooE2ee {
  private readonly outbound = new Map<string, Outbound>();
  private readonly queues = new Map<string, Promise<unknown>>();
  private readonly processed: Set<string>;
  private readonly loadedInbound = new Set<string>();
  private syncing: Promise<number> | undefined;
  /** The store could not be saved after the memory moved ahead; the device must be opened again. */
  private broken = false;
  private saving: Promise<void> = Promise.resolve();

  private readonly options: E2eeOptions;
  private readonly state: StoredState;
  private readonly account: DeviceAccount;
  private readonly ownKeys: OwnKeys;
  private readonly trust: TrustStore;
  private readonly olm: OlmSessions;
  private readonly inbound: InboundSessionStore;

  private constructor(options: E2eeOptions, state: StoredState, account: DeviceAccount, ownKeys: OwnKeys, trust: TrustStore, olm: OlmSessions, inbound: InboundSessionStore) {
    this.options = options;
    this.state = state;
    this.account = account;
    this.ownKeys = ownKeys;
    this.trust = trust;
    this.olm = olm;
    this.inbound = inbound;
    this.processed = new Set(state.processed_to_device);
  }

  get userId(): string {
    return this.state.user_id;
  }

  get deviceId(): string {
    return this.state.device_id;
  }

  /**
   * Open the bot's device: restore it from the store, or set up a new device (register it, upload
   * new cross-signing keys, sign the device) and remove the bot's other devices.
   */
  static async open(options: E2eeOptions): Promise<ZooE2ee> {
    if (options.storageKey.length !== 32) throw new Error("storageKey must be 32 bytes");
    const me = await options.app.request<{ bot_user_id: string; user_id: string; delegated: boolean; scopes: string[] }>("GET", "/auth");
    if (me.delegated || me.user_id !== me.bot_user_id) {
      throw new Error("An E2EE device needs the bot's own token, not a delegated token");
    }
    if (!me.scopes.includes("e2ee:device")) throw new Error("The app token lacks the e2ee:device scope");
    const raw = await options.store.load();
    const e2ee = raw ? ZooE2ee.restore(options, JSON.parse(raw) as StoredState) : await ZooE2ee.setUp(options, me.bot_user_id);
    if (e2ee.userId !== me.bot_user_id) throw new Error("The E2EE store belongs to another bot");
    await e2ee.replenishKeys();
    return e2ee;
  }

  private static restore(options: E2eeOptions, state: StoredState): ZooE2ee {
    const { crypto, storageKey } = options;
    if (state.version !== 1) throw new Error(`Unsupported E2EE store version: ${String(state.version)}`);
    const account = crypto.DeviceAccount.fromPickle(state.account, storageKey, state.user_id, state.device_id);
    const ownKeys = crypto.OwnKeys.fromPickle(state.own_keys, account, storageKey);
    const trust = crypto.TrustStore.fromPickle(state.trust, account, storageKey);
    const olm = new crypto.OlmSessions();
    for (const session of state.olm_sessions) olm.pushPickle(session.pickle, account, storageKey);
    return new ZooE2ee(options, state, account, ownKeys, trust, olm, new crypto.InboundSessionStore());
  }

  private static async setUp(options: E2eeOptions, userId: string): Promise<ZooE2ee> {
    const { app, crypto, storageKey } = options;
    const deviceId = randomUUID();
    const account = new crypto.DeviceAccount(userId, deviceId);
    account.generateOneTimeKeys(ONE_TIME_KEY_TARGET);
    account.generateFallbackKey();
    const deviceKeys = account.deviceKeys();
    await app.request("POST", "/e2ee/devices", {
      device_keys: deviceKeys,
      display_name: options.deviceName ?? "Bot",
      one_time_keys: account.oneTimeKeys(),
      fallback_keys: account.fallbackKeys(),
    });
    account.markKeysAsPublished();

    // Replace the cross-signing keys of a previous setup whose store was lost.
    const query = await app.request<{ master_keys?: Record<string, JsonObject>; self_signing_keys?: Record<string, JsonObject>; user_signing_keys?: Record<string, JsonObject> }>("POST", "/e2ee/keys/query", { user_ids: [userId] });
    const publicKey = (key: JsonObject | undefined) => (key ? (Object.values((key.keys ?? {}) as Record<string, string>)[0] ?? null) : null);
    const current = {
      master: publicKey(query.master_keys?.[userId]),
      self_signing: publicKey(query.self_signing_keys?.[userId]),
      user_signing: publicKey(query.user_signing_keys?.[userId]),
    };
    const secrets = crypto.OwnSecrets.generate(userId);
    const crossSigning = secrets.crossSigning;
    const trust = new crypto.TrustStore(userId);
    try {
      await app.request("POST", "/e2ee/cross_signing/keys", {
        ...crossSigning.uploadJson(),
        ...(current.master ? { expected_keys: current } : {}),
      });
      const master = crossSigning.master;
      const userSigning = crossSigning.userSigning;
      const selfSigning = crossSigning.selfSigning;
      try {
        trust.trustOwnMasterKey(master);
        trust.recordOwnUserSigningKey(userSigning);
        await app.request("POST", "/e2ee/cross_signing/signatures", { signatures: [selfSigning.signDevice(deviceKeys)] });
      } finally {
        master.free();
        userSigning.free();
        selfSigning.free();
      }
    } finally {
      crossSigning.free();
    }
    // The master key's private key is not kept (docs/e2ee.md, "Bot の端末").
    const ownKeys = secrets.ownKeys();
    secrets.free();
    const state: StoredState = {
      version: 1,
      user_id: userId,
      device_id: deviceId,
      account: account.pickle(storageKey),
      own_keys: ownKeys.pickle(account, storageKey),
      trust: trust.pickle(account, storageKey),
      olm_sessions: [],
      outbound_olm: {},
      inbound: {},
      processed_to_device: [],
    };
    await options.store.save(JSON.stringify(state));
    const e2ee = new ZooE2ee(options, state, account, ownKeys, trust, new crypto.OlmSessions(), new crypto.InboundSessionStore());
    // A bot has one device. Devices of earlier setups can no longer decrypt anything; remove them.
    const devices = await app.request<{ device_id: string }[]>("GET", "/e2ee/devices");
    for (const device of devices) {
      if (device.device_id !== deviceId) await app.request("DELETE", `/e2ee/devices/${encodeURIComponent(device.device_id)}`);
    }
    return e2ee;
  }

  /** Keep {@link ONE_TIME_KEY_TARGET} one-time keys and an unused fallback key on the server. */
  async replenishKeys(): Promise<void> {
    const counts = await this.options.app.request<{ one_time_key_counts: { signed_curve25519: number }; unused_fallback_key_types: string[] }>("POST", "/e2ee/keys/upload", { device_id: this.deviceId });
    const missing = ONE_TIME_KEY_TARGET - counts.one_time_key_counts.signed_curve25519;
    const needsFallback = !counts.unused_fallback_key_types.includes("signed_curve25519");
    if (missing <= 0 && !needsFallback) return;
    if (missing > 0) this.account.generateOneTimeKeys(missing);
    if (needsFallback) this.account.generateFallbackKey();
    await this.save();
    await this.options.app.request("POST", "/e2ee/keys/upload", {
      device_id: this.deviceId,
      one_time_keys: this.account.oneTimeKeys(),
      fallback_keys: needsFallback ? this.account.fallbackKeys() : {},
    });
    this.account.markKeysAsPublished();
    await this.save();
  }

  /**
   * Fetch and process the to-device messages for this device (Megolm session keys), then ACK them.
   * Returns the number of session keys stored. Concurrent calls share one run.
   */
  syncToDevice(): Promise<number> {
    this.syncing ??= this.runSync().finally(() => {
      this.syncing = undefined;
    });
    return this.syncing;
  }

  private async runSync(): Promise<number> {
    const { app } = this.options;
    if (this.broken) throw new Error("The E2EE store could not be saved; open the device again");
    let stored = 0;
    let newOlmSessions = false;
    for (;;) {
      const { messages } = await app.request<{ messages: ToDeviceRow[] }>("GET", `/e2ee/to_device?device_id=${encodeURIComponent(this.deviceId)}&limit=${TO_DEVICE_PAGE}`);
      if (messages.length === 0) break;
      for (const row of messages) {
        if (this.processed.has(row.id)) continue;
        const before = this.olm.length;
        stored += await this.processToDevice(row);
        if (this.olm.length > before) newOlmSessions = true;
        try {
          // The advanced Olm state, the session key and the processed mark are saved together.
          await this.save();
        } catch (error) {
          // The memory is ahead of the store. Continuing would ACK a message whose Olm state was never
          // saved; after a restart from the store the message can be decrypted again (as the web
          // client's crypto store does, frontend/src/lib/e2ee/store/cryptoStore.ts).
          this.broken = true;
          throw error;
        }
      }
      await app.request("POST", "/e2ee/to_device/ack", { device_id: this.deviceId, ids: messages.map((row) => row.id) });
      if (messages.length < TO_DEVICE_PAGE) break;
    }
    // Inbound Olm sessions used one-time keys.
    if (newOlmSessions) await this.replenishKeys();
    return stored;
  }

  /**
   * Decrypt and verify one to-device message and keep its session key in memory (the caller saves).
   *
   * An Olm message can be decrypted only once, so nothing after `decryptToDevice` may throw:
   * network requests that can fail come first (the sender's keys from `keys/query`), and when the
   * channel's snapshot cannot be read after a few attempts, the key is kept with an unverified sender
   * instead of being lost.
   */
  private async processToDevice(row: ToDeviceRow): Promise<number> {
    if (row.type !== "m.room.encrypted") {
      this.markProcessed(row.id);
      return 0;
    }
    const { storageKey } = this.options;
    // Before decrypting: the device keys of the sender the server reports (errors leave the message
    // in the queue for the next sync).
    const query = await this.options.app.request<{ device_keys?: Record<string, Record<string, JsonObject>> }>("POST", "/e2ee/keys/query", { user_ids: [row.sender_user_id] });
    let decrypted;
    try {
      decrypted = this.account.decryptToDevice(this.olm, row.content);
    } catch {
      this.markProcessed(row.id);
      return 0; // An undecryptable message never becomes readable; drop it.
    }
    // Decryption advanced the account and the Olm sessions. Their pickles go into the stored state
    // only together with the processed mark and the session key (the `finally` below, which runs
    // without awaiting), so a save from a concurrent send never stores one without the others.
    const account = this.account.pickle(storageKey);
    const pickled = this.olm.pickleSessions(this.account, storageKey, decrypted.senderKey) as StoredState["olm_sessions"];
    let inbound: [string, string] | null = null;
    try {
      const sender = decrypted.sender;
      const senderDevice = decrypted.senderDevice;
      const plaintext = decrypted.plaintext as { content?: { channel_id?: unknown } };
      const channelId = plaintext.content?.channel_id;
      if (!sender || !senderDevice || typeof channelId !== "string") return 0;
      let listed: JsonObject | null = sender === row.sender_user_id ? (query.device_keys?.[sender]?.[senderDevice] ?? null) : null;
      // The sender's trust from the channel's snapshot (members only), with retries.
      let trust: { self_signing_key: string; signature: string | null } | null = null;
      const sleep = this.options.sleep ?? ((ms: number) => new Promise((resolve) => setTimeout(resolve, ms)));
      for (let attempt = 0; ; attempt += 1) {
        try {
          // Not saved here: the Olm state, the key and the processed mark are saved together.
          const evaluation = await this.evaluate(channelId, null, false);
          try {
            listed = deviceKeysOf(evaluation.pages, sender, senderDevice) ?? listed;
            // Only a device the evaluation verified (trusted master key → self-signing key signed by
            // it → device signed by that) gets the sender's self-signing key. The user's state alone
            // is not enough: a user stays trusted when the server swaps in a self-signing key that the
            // master key never signed, and zoo-crypto checks only the device's signature by the key
            // it is given.
            const verified = evaluation.devices.some((device) => device.userId === sender && device.deviceId === senderDevice && device.curve25519 === decrypted.senderKey);
            if (verified) trust = senderTrust(evaluation.pages, sender, senderDevice);
          } finally {
            freeEvaluation(evaluation);
          }
          break;
        } catch (error) {
          // Not a member (anymore, or not allowed): the key is useless.
          if (error instanceof ZooError && (error.status === 403 || error.status === 404)) return 0;
          // Keep the key with an unverified sender rather than lose it.
          if (attempt >= EVALUATE_RETRY_MS.length) break;
          await sleep(EVALUATE_RETRY_MS[attempt]!);
        }
      }
      let roomKey;
      try {
        roomKey = decrypted.verifyRoomKey(this.account, listed, trust);
      } catch {
        return 0;
      }
      const ref = { channel_id: roomKey.channelId, sender_key: roomKey.senderKey, session_id: roomKey.sessionId };
      this.loadInbound(ref);
      const outcome = this.inbound.addRoomKey(roomKey);
      if (outcome === "kept") return 0;
      const pickle = this.inbound.pickleSession(ref, this.account, storageKey);
      if (pickle) inbound = [inboundKey(ref), pickle];
      return 1;
    } finally {
      const senderKey = decrypted.senderKey;
      decrypted.free();
      this.state.account = account;
      this.state.olm_sessions = [...this.state.olm_sessions.filter((session) => session.their_identity_key !== senderKey), ...pickled];
      if (inbound) this.state.inbound[inbound[0]] = inbound[1];
      this.markProcessed(row.id);
    }
  }

  private markProcessed(id: string): void {
    this.processed.add(id);
    this.state.processed_to_device = [...this.processed].slice(-PROCESSED_TO_DEVICE_MEMORY);
  }

  private loadInbound(ref: { channel_id: string; sender_key: string; session_id: string }): void {
    const key = inboundKey(ref);
    if (this.loadedInbound.has(key)) return;
    const pickle = this.state.inbound[key];
    if (pickle) this.inbound.restoreSession(pickle, this.account, this.options.storageKey);
    this.loadedInbound.add(key);
  }

  /**
   * Decrypt a `message.created` or `message.updated` event of an encrypted channel. Returns `null` for
   * other events. Waits for the session key (to-device) briefly, then throws so the event is retried.
   */
  async decryptEvent(event: AppEvent): Promise<DecryptedMessage | null> {
    if (event.type !== "message.created" && event.type !== "message.updated") return null;
    const data = event.data as { id?: string; channel_id?: string; parent_id?: string | null; user_id?: string; message_kind?: string; epoch?: number | string; revision?: number | string; ciphertext?: JsonObject; encrypted?: boolean };
    if (data.encrypted !== true || !data.id || !data.channel_id || !data.user_id || !data.ciphertext) return null;
    const ciphertext = data.ciphertext as { sender_key?: unknown; session_id?: unknown };
    if (typeof ciphertext.sender_key !== "string" || typeof ciphertext.session_id !== "string") {
      throw new Error("The encrypted message has no session reference");
    }
    const ref = { channel_id: data.channel_id, sender_key: ciphertext.sender_key, session_id: ciphertext.session_id };
    const row = {
      channel_id: data.channel_id,
      message_id: data.id,
      parent_id: data.parent_id ?? null,
      user_id: data.user_id,
      message_kind: data.message_kind ?? "message",
      epoch: Number(data.epoch ?? 0),
      revision: Number(data.revision ?? 0),
      ciphertext: data.ciphertext,
    };
    const sleep = this.options.sleep ?? ((ms: number) => new Promise((resolve) => setTimeout(resolve, ms)));
    for (let attempt = 0; ; attempt += 1) {
      this.loadInbound(ref);
      try {
        const decrypted = this.inbound.decrypt(row);
        const payload = decrypted.payload;
        return {
          id: payload.message_id,
          channelId: payload.channel_id,
          parentId: payload.parent_id,
          userId: row.user_id,
          body: payload.content.body,
          kind: payload.content.kind === "emote" ? "emote" : "message",
          mentions: payload.mentions,
          revision: payload.revision,
          senderVerified: decrypted.origin === "olm" && decrypted.sender_verified,
        };
      } catch (error) {
        if ((error as { code?: unknown }).code !== "MissingRoomKey" || attempt >= KEY_WAIT_MS.length) throw error;
      }
      // The sender shares the key before posting, but the event can arrive first.
      if ((await this.syncToDevice()) === 0) await sleep(KEY_WAIT_MS[attempt]!);
    }
  }

  /**
   * Encrypt and post a message to an encrypted channel the bot was allowed into. Returns its id.
   * The session key is shared first with every verified device of the channel's current snapshot.
   */
  async sendMessage(channelId: string, body: string, options: { parentId?: string | null; kind?: "message" | "emote"; mentions?: Mentions; id?: string } = {}): Promise<string> {
    const id = options.id ?? randomUUID();
    const mentions = options.mentions ?? { user_ids: [], channel: false, here: false };
    const kind = options.kind ?? "message";
    const message = { message_id: id, parent_id: options.parentId ?? null, revision: 0, content: { body, format: "mfm", kind }, mentions };
    let minEpoch: number | null = null;
    for (let attempt = 0; attempt < STALE_EPOCH_ATTEMPTS; attempt += 1) {
      const { ciphertext, epoch } = await this.serialized(channelId, async () => {
        const current = await this.prepare(channelId, minEpoch);
        return { ciphertext: current.session.encrypt(this.account, message), epoch: current.session.epoch };
      });
      try {
        await this.options.app.request("POST", "/messages", {
          id,
          channel_id: channelId,
          content: "",
          kind,
          epoch,
          revision: 0,
          ciphertext,
          mentions,
          ...(options.parentId ? { parent_id: options.parentId } : {}),
        });
        return id;
      } catch (error) {
        if (!(error instanceof ZooError) || error.code !== "E2EE_STALE_EPOCH") throw error;
        const current = Number((error.body as { current_epoch?: unknown }).current_epoch);
        minEpoch = Number.isFinite(current) ? current : epoch + 1;
      }
    }
    throw new E2eeSendError("The channel's membership kept changing; the message was not sent");
  }

  /** Run tasks of one channel one at a time (sharing a session and encrypting with it). */
  private serialized<T>(channelId: string, task: () => Promise<T>): Promise<T> {
    const previous = this.queues.get(channelId) ?? Promise.resolve();
    const next = previous.catch(() => undefined).then(task);
    this.queues.set(channelId, next);
    void next.finally(() => {
      if (this.queues.get(channelId) === next) this.queues.delete(channelId);
    }).catch(() => undefined);
    return next;
  }

  /**
   * Prepare the outbound Megolm session for the current snapshot (a new one when the epoch changed or
   * the session needs rotation) and share its key with the verified devices that do not have it yet.
   */
  private async prepare(channelId: string, minEpoch: number | null): Promise<Outbound> {
    const { crypto, app, storageKey } = this.options;
    const now = (this.options.now ?? Date.now)();
    const evaluation = await this.evaluate(channelId, minEpoch);
    try {
      let current = this.outbound.get(channelId);
      if (current && (current.session.epoch !== evaluation.epoch || current.session.needsRotation(now, evaluation.epoch))) {
        current.session.free();
        this.outbound.delete(channelId);
        current = undefined;
      }
      if (!current) {
        current = { session: new crypto.OutboundSession(channelId, evaluation.epoch, now), shared: new Set() };
        this.outbound.set(channelId, current);
      }
      const pending = evaluation.devices.filter((device) => !current.shared.has(`${device.userId}|${device.deviceId}`));
      if (pending.length === 0) return current;
      try {
        // One-time keys for the devices without an outbound Olm session.
        const claim: Record<string, Record<string, "signed_curve25519">> = {};
        for (const device of pending) {
          if (!this.state.outbound_olm[device.curve25519]) (claim[device.userId] ??= {})[device.deviceId] = "signed_curve25519";
        }
        const claimed = await this.claim(claim);
        const roomKey = current.session.roomKey();
        const messages: Record<string, Record<string, JsonObject>> = {};
        try {
          for (const device of pending) {
            const stored = this.state.outbound_olm[device.curve25519];
            let olm;
            if (stored) {
              olm = crypto.OlmSession.fromPickle(stored, this.account, storageKey);
            } else {
              const oneTimeKey = Object.values(claimed[device.userId]?.[device.deviceId] ?? {})[0];
              if (!oneTimeKey) continue; // No key to claim; retried on the next send.
              olm = this.account.createOutboundOlmSession(device, oneTimeKey);
            }
            try {
              (messages[device.userId] ??= {})[device.deviceId] = this.account.encryptRoomKey(olm, device, roomKey);
              this.state.outbound_olm[device.curve25519] = olm.pickle(this.account, storageKey);
            } finally {
              olm.free();
            }
          }
        } finally {
          roomKey.free();
        }
        await this.save();
        const undelivered = await this.sendToDevice(messages);
        for (const [userId, devices] of Object.entries(messages)) {
          for (const deviceId of Object.keys(devices)) {
            if (!undelivered.has(`${userId}|${deviceId}`)) current.shared.add(`${userId}|${deviceId}`);
          }
        }
        return current;
      } catch (error) {
        // Never encrypt with a session whose key may not have reached anyone.
        current.session.free();
        this.outbound.delete(channelId);
        throw error;
      }
    } finally {
      freeEvaluation(evaluation);
    }
  }

  private async claim(claim: Record<string, Record<string, "signed_curve25519">>): Promise<Record<string, Record<string, Record<string, JsonObject>>>> {
    const entries = Object.entries(claim).flatMap(([userId, devices]) => Object.keys(devices).map((deviceId) => [userId, deviceId] as const));
    const claimed: Record<string, Record<string, Record<string, JsonObject>>> = {};
    for (let start = 0; start < entries.length; start += KEYS_CLAIM_BATCH) {
      const batch: Record<string, Record<string, string>> = {};
      for (const [userId, deviceId] of entries.slice(start, start + KEYS_CLAIM_BATCH)) (batch[userId] ??= {})[deviceId] = "signed_curve25519";
      const response = await this.options.app.request<{ one_time_keys?: typeof claimed }>("POST", "/e2ee/keys/claim", { one_time_keys: batch });
      for (const [userId, devices] of Object.entries(response.one_time_keys ?? {})) Object.assign((claimed[userId] ??= {}), devices);
    }
    return claimed;
  }

  private async sendToDevice(messages: Record<string, Record<string, JsonObject>>): Promise<Set<string>> {
    const entries = Object.entries(messages).flatMap(([userId, devices]) => Object.entries(devices).map(([deviceId, content]) => ({ userId, deviceId, content })));
    const undelivered = new Set<string>();
    for (let start = 0; start < entries.length; start += TO_DEVICE_BATCH) {
      const batch: Record<string, Record<string, JsonObject>> = {};
      for (const entry of entries.slice(start, start + TO_DEVICE_BATCH)) (batch[entry.userId] ??= {})[entry.deviceId] = entry.content;
      const response = await this.options.app.request<{ undelivered?: { user_id: string; device_id: string }[] }>("POST", "/e2ee/to_device", {
        device_id: this.deviceId,
        txn_id: randomUUID(),
        type: "m.room.encrypted",
        messages: batch,
      });
      for (const target of response.undelivered ?? []) undelivered.add(`${target.user_id}|${target.device_id}`);
    }
    return undelivered;
  }

  /**
   * Fetch the channel's snapshot (all pages, one epoch) and decide which devices may receive keys
   * (docs/e2ee.md, "鍵の配布先の判定"): TOFU for members seen for the first time, nothing for members
   * whose master key changed. New TOFU pins are stored on the server.
   */
  private async evaluate(channelId: string, minEpoch: number | null, persist = true): Promise<Evaluation> {
    const { crypto } = this.options;
    let pages: JsonObject[] | null = null;
    for (let round = 0; ; round += 1) {
      // After new TOFU pins are stored, the same snapshot is evaluated again: users whose pin was just
      // made are `pin_pending` in the first evaluation and get no keys until their pin is confirmed.
      pages ??= await this.fetchRecipients(channelId, minEpoch);
      const epoch = Number(pages[0]!.membership_epoch);
      const snapshot = crypto.KeySnapshot.fromJson(pages[0]!);
      const userSigning = this.ownKeys.userSigning;
      let distribution;
      try {
        for (const page of pages.slice(1)) snapshot.extend(crypto.KeySnapshot.fromJson(page));
        distribution = this.trust.evaluateWithUserSigning(snapshot, this.deviceId, userSigning);
      } finally {
        userSigning.free();
        snapshot.free();
      }
      let conflict = false;
      const newPins = distribution.new_pins.length > 0;
      for (const pin of distribution.new_pins) {
        try {
          await this.options.app.request("POST", "/e2ee/tofu_pins", { pins: [pin.pin], replace: false });
          this.trust.confirmTofuPin(pin);
        } catch (error) {
          if (!(error instanceof ZooError) || error.code !== "E2EE_PIN_CONFLICT") {
            for (const device of distribution.devices) device.free();
            throw error;
          }
          this.trust.discardTofuPin(pin);
          conflict = true;
        }
      }
      this.state.trust = this.trust.pickle(this.account, this.options.storageKey);
      if (persist) await this.save();
      if (newPins && round < 2) {
        for (const device of distribution.devices) device.free();
        // A conflicting pin (another device of the bot stored one first) needs a fresh snapshot.
        if (conflict) pages = null;
        continue;
      }
      return { epoch, pages, devices: distribution.devices };
    }
  }

  private async fetchRecipients(channelId: string, minEpoch: number | null): Promise<JsonObject[]> {
    const base = `/e2ee/channels/${encodeURIComponent(channelId)}/recipients`;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const pages: JsonObject[] = [];
      let epoch: string | null = null;
      let after: string | null = null;
      try {
        do {
          const params = new URLSearchParams();
          if (after !== null) params.set("after", after);
          if (epoch !== null) params.set("epoch", epoch);
          const page: JsonObject = await this.options.app.request<JsonObject>("GET", params.size > 0 ? `${base}?${params}` : base);
          pages.push(page);
          epoch ??= String(page.membership_epoch ?? 0);
          after = typeof page.next_after === "string" ? page.next_after : null;
        } while (after !== null);
      } catch (error) {
        if (error instanceof ZooError && error.code === "E2EE_STALE_EPOCH") continue;
        throw error;
      }
      if (minEpoch !== null && Number(epoch) < minEpoch) continue;
      return pages;
    }
    throw new Error("Could not read the channel's recipients");
  }

  /** Save the state. Saves are serialized and always write the latest state. */
  private save(): Promise<void> {
    this.saving = this.saving.catch(() => undefined).then(() => this.options.store.save(JSON.stringify(this.state)));
    return this.saving;
  }

  /** Free the wasm objects. The instance cannot be used afterwards. */
  close(): void {
    for (const current of this.outbound.values()) current.session.free();
    this.outbound.clear();
    this.inbound.free();
    this.olm.free();
    this.trust.free();
    this.ownKeys.free();
    this.account.free();
  }
}

function freeEvaluation(evaluation: Evaluation): void {
  for (const device of evaluation.devices) device.free();
}

/** The sender's device keys in the snapshot (as registered, with its self-signature). */
function deviceKeysOf(pages: JsonObject[], userId: string, deviceId: string): JsonObject | null {
  for (const page of pages) {
    const devices = (page.device_keys as Record<string, Record<string, JsonObject>> | undefined)?.[userId];
    if (devices?.[deviceId]) return devices[deviceId];
  }
  return null;
}

/**
 * The sender's self-signing key and its signature on the device, for a sender whose master key the
 * bot trusts. `zoo-crypto` checks both again before marking the session's sender as verified.
 */
function senderTrust(pages: JsonObject[], userId: string, deviceId: string): { self_signing_key: string; signature: string | null } | null {
  for (const page of pages) {
    const key = (page.self_signing_keys as Record<string, { keys?: Record<string, string> }> | undefined)?.[userId];
    const selfSigningKey = key?.keys ? Object.values(key.keys)[0] : undefined;
    if (!selfSigningKey) continue;
    const signatures = (page.signatures ?? []) as { signer_user_id: string; signer_key_id: string; target_user_id: string; target_key_id: string; signature: string }[];
    const signature = signatures.find((s) => s.signer_user_id === userId && s.target_user_id === userId && s.target_key_id === deviceId && s.signer_key_id === `ed25519:${selfSigningKey}`);
    return { self_signing_key: selfSigningKey, signature: signature?.signature ?? null };
  }
  return null;
}
