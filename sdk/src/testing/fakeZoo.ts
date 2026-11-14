/**
 * An in-memory stand-in for the parts of Zoo that an E2EE bot uses, and a "human" device built
 * directly on zoo-crypto, the way the web client works (frontend/src/lib/e2ee). Tests only.
 *
 * The response shapes follow the server (backend/src/routes/e2ee*.rs and backend/src/apps/e2ee.rs):
 * `recipients` pages, `keys/query`, `keys/claim`, to-device rows and the encrypted message events
 * that `app_message_events_v2` emits to bots allowed into an encrypted channel.
 */
import { randomUUID } from "node:crypto";

import type { AppEvent } from "../index.ts";
import type { ZooCrypto } from "../e2ee.ts";

type JsonObject = import("../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm.js").JsonObject;
type Wasm = ZooCrypto;

interface Signature {
  signer_user_id: string;
  signer_key_id: string;
  target_user_id: string;
  target_key_id: string;
  signature: string;
}

interface ToDevice {
  id: string;
  recipient_user_id: string;
  recipient_device_id: string;
  sender_user_id: string;
  sender_device_id: string;
  type: string;
  content: JsonObject;
}

export interface StoredMessage {
  id: string;
  channel_id: string;
  parent_id: string | null;
  user_id: string;
  message_kind: string;
  epoch: number;
  revision: number;
  ciphertext: JsonObject;
}

class HttpError extends Error {
  readonly status: number;
  readonly body: JsonObject;
  constructor(status: number, body: JsonObject) {
    super(String(body.error));
    this.status = status;
    this.body = body;
  }
}

const publicKeyOf = (key: JsonObject | undefined): string | undefined =>
  key ? Object.values((key.keys ?? {}) as Record<string, string>)[0] : undefined;

export class FakeZoo {
  readonly devices = new Map<string, Map<string, JsonObject>>();
  readonly deletedDevices = new Set<string>();
  readonly oneTimeKeys = new Map<string, Map<string, JsonObject>>();
  readonly fallbackKeys = new Map<string, JsonObject>();
  readonly crossSigning = new Map<string, Partial<Record<"master" | "self_signing" | "user_signing", JsonObject>>>();
  readonly signatures: Signature[] = [];
  readonly pins = new Map<string, JsonObject>();
  readonly toDevice: ToDevice[] = [];
  readonly channels = new Map<string, { epoch: number; members: Set<string>; botAllowed: Set<string> }>();
  readonly messages: StoredMessage[] = [];
  /** Events delivered to the bot (only from members other than the bot, as app_emit suppresses the bot's own). */
  readonly botEvents: AppEvent[] = [];
  private nextToDevice = 1;
  readonly botUserId: string;
  readonly scopes: string[];

  constructor(botUserId: string, scopes: string[] = ["messages:read", "messages:write", "channels:manage", "e2ee:device"]) {
    this.botUserId = botUserId;
    this.scopes = scopes;
  }

  channel(members: string[], botAllowed = true): string {
    const id = randomUUID();
    this.channels.set(id, {
      epoch: 1,
      members: new Set(members),
      botAllowed: new Set(botAllowed ? members.filter((m) => m === this.botUserId) : []),
    });
    return id;
  }

  bumpEpoch(channelId: string): void {
    this.channels.get(channelId)!.epoch += 1;
  }

  private bumpUser(userId: string): void {
    for (const channel of this.channels.values()) if (channel.members.has(userId)) channel.epoch += 1;
  }

  // ---------------------------------------------------------------- the bot's HTTP API

  readonly fetch: typeof fetch = async (input, init) => {
    const url = new URL(String(input));
    const method = init?.method ?? "GET";
    const body = init?.body ? (JSON.parse(String(init.body)) as JsonObject) : {};
    const path = url.pathname.replace(/^\/api\/apps\/v1/, "");
    try {
      const data = this.route(method, path, url.searchParams, body);
      return Response.json(data);
    } catch (error) {
      if (error instanceof HttpError) return Response.json(error.body, { status: error.status });
      throw error;
    }
  };

  private route(method: string, path: string, params: URLSearchParams, body: JsonObject): unknown {
    const bot = this.botUserId;
    const wrap = (data: unknown) => ({ data, txid: "1" });
    let match: RegExpMatchArray | null;
    if (method === "GET" && path === "/auth") {
      return { app_id: "app", user_id: bot, bot_user_id: bot, delegated: false, scopes: this.scopes };
    }
    if (method === "POST" && path === "/e2ee/devices") return wrap(this.registerDevice(bot, body));
    if (method === "GET" && path === "/e2ee/devices") {
      return [...(this.devices.get(bot)?.keys() ?? [])].map((device_id) => ({ device_id }));
    }
    if (method === "DELETE" && (match = path.match(/^\/e2ee\/devices\/([^/]+)$/))) {
      this.deleteDevice(bot, decodeURIComponent(match[1]!));
      return wrap({ device_id: match[1] });
    }
    if (method === "POST" && path === "/e2ee/keys/upload") return wrap(this.uploadKeys(bot, body));
    if (method === "POST" && path === "/e2ee/keys/query") return this.queryKeys(bot, body.user_ids as string[]);
    if (method === "POST" && path === "/e2ee/keys/claim") return wrap({ one_time_keys: this.claim(body.one_time_keys as Record<string, Record<string, string>>) });
    if (method === "POST" && path === "/e2ee/cross_signing/keys") return wrap(this.uploadCrossSigning(bot, body));
    if (method === "POST" && path === "/e2ee/cross_signing/signatures") {
      for (const signature of body.signatures as Omit<Signature, "signer_user_id">[]) this.signatures.push({ ...signature, signer_user_id: bot });
      return wrap({});
    }
    if (method === "POST" && path === "/e2ee/tofu_pins") return wrap(this.uploadPins(bot, body.pins as JsonObject[]));
    if (method === "GET" && (match = path.match(/^\/e2ee\/channels\/([^/]+)\/recipients$/))) {
      const channelId = decodeURIComponent(match[1]!);
      const channel = this.channels.get(channelId);
      if (!channel?.members.has(bot)) throw new HttpError(404, { error: "Channel not found" });
      if (!channel.botAllowed.has(bot)) throw new HttpError(403, { error: "Apps cannot use encrypted channels", code: "E2EE_APPS_FORBIDDEN" });
      const epoch = params.get("epoch");
      if (epoch !== null && Number(epoch) !== channel.epoch) throw new HttpError(409, { error: "stale", code: "E2EE_STALE_EPOCH", current_epoch: channel.epoch });
      return this.recipients(channelId, bot);
    }
    if (method === "GET" && path === "/e2ee/to_device") return { messages: this.pendingToDevice(bot, params.get("device_id")!) };
    if (method === "POST" && path === "/e2ee/to_device/ack") {
      this.ack(bot, body.device_id as string, body.ids as string[]);
      return wrap({});
    }
    if (method === "POST" && path === "/e2ee/to_device") return wrap(this.sendToDevice(bot, body));
    if (method === "POST" && path === "/messages") return wrap(this.postMessage(bot, body));
    throw new HttpError(404, { error: `No route: ${method} ${path}` });
  }

  // ---------------------------------------------------------------- shared server behaviour

  registerDevice(userId: string, body: JsonObject): JsonObject {
    const keys = body.device_keys as JsonObject;
    const deviceId = keys.device_id as string;
    let devices = this.devices.get(userId);
    if (!devices) this.devices.set(userId, (devices = new Map()));
    devices.set(deviceId, keys);
    return this.uploadKeys(userId, { device_id: deviceId, one_time_keys: body.one_time_keys, fallback_keys: body.fallback_keys });
  }

  deleteDevice(userId: string, deviceId: string): void {
    if (this.devices.get(userId)?.delete(deviceId)) {
      this.deletedDevices.add(`${userId}|${deviceId}`);
      this.bumpUser(userId);
    }
  }

  uploadKeys(userId: string, body: JsonObject): JsonObject {
    const id = `${userId}|${String(body.device_id)}`;
    let keys = this.oneTimeKeys.get(id);
    if (!keys) this.oneTimeKeys.set(id, (keys = new Map()));
    for (const [keyId, key] of Object.entries((body.one_time_keys ?? {}) as Record<string, JsonObject>)) keys.set(keyId, key);
    for (const [keyId, key] of Object.entries((body.fallback_keys ?? {}) as Record<string, JsonObject>)) this.fallbackKeys.set(id, { [keyId]: key });
    return {
      one_time_key_counts: { signed_curve25519: keys.size },
      unused_fallback_key_types: this.fallbackKeys.has(id) ? ["signed_curve25519"] : [],
    };
  }

  queryKeys(caller: string, userIds: string[]): JsonObject {
    const result: Record<string, Record<string, unknown>> = { device_keys: {}, master_keys: {}, self_signing_keys: {}, user_signing_keys: {} };
    for (const userId of userIds) {
      result.device_keys![userId] = Object.fromEntries(this.devices.get(userId) ?? []);
      const keys = this.crossSigning.get(userId);
      if (keys?.master) result.master_keys![userId] = keys.master;
      if (keys?.self_signing) result.self_signing_keys![userId] = keys.self_signing;
      if (keys?.user_signing && userId === caller) result.user_signing_keys![userId] = keys.user_signing;
    }
    return result as JsonObject;
  }

  claim(request: Record<string, Record<string, string>>): Record<string, Record<string, JsonObject>> {
    const result: Record<string, Record<string, JsonObject>> = {};
    for (const [userId, devices] of Object.entries(request)) {
      for (const deviceId of Object.keys(devices)) {
        const id = `${userId}|${deviceId}`;
        const keys = this.oneTimeKeys.get(id);
        const first = keys?.entries().next().value as [string, JsonObject] | undefined;
        if (first) {
          keys!.delete(first[0]);
          (result[userId] ??= {})[deviceId] = { [first[0]]: first[1] };
        } else if (this.fallbackKeys.has(id)) {
          (result[userId] ??= {})[deviceId] = this.fallbackKeys.get(id)!;
        }
      }
    }
    return result;
  }

  uploadCrossSigning(userId: string, body: JsonObject): JsonObject {
    const current = this.crossSigning.get(userId) ?? {};
    if (current.master && !body.expected_keys) throw new HttpError(400, { error: "expected_keys is required unless no key is registered" });
    const next = { ...current };
    if (body.master_key) next.master = body.master_key as JsonObject;
    if (body.self_signing_key) next.self_signing = body.self_signing_key as JsonObject;
    if (body.user_signing_key) next.user_signing = body.user_signing_key as JsonObject;
    if (publicKeyOf(current.master) !== publicKeyOf(next.master)) {
      // Signatures made by or on the old keys are gone (backend: upload_cross_signing_keys).
      for (let i = this.signatures.length - 1; i >= 0; i -= 1) if (this.signatures[i]!.signer_user_id === userId) this.signatures.splice(i, 1);
    }
    this.crossSigning.set(userId, next);
    this.bumpUser(userId);
    return {};
  }

  uploadPins(owner: string, pins: JsonObject[]): JsonObject {
    for (const pin of pins) {
      const key = `${owner}|${String(pin.target_user_id)}`;
      const existing = this.pins.get(key);
      if (existing && existing.master_key !== pin.master_key) throw new HttpError(409, { error: "conflict", code: "E2EE_PIN_CONFLICT", pin: existing });
      this.pins.set(key, pin);
    }
    return {};
  }

  /** One page of `GET /api/e2ee/channels/{id}/recipients` (backend: get_recipients). */
  recipients(channelId: string, caller: string): JsonObject {
    const channel = this.channels.get(channelId)!;
    const members = [...channel.members].sort();
    const page: Record<string, unknown> = {
      channel_id: channelId,
      membership_epoch: channel.epoch,
      user_ids: members,
      device_keys: {},
      master_keys: {},
      self_signing_keys: {},
      user_signing_keys: {},
      signatures: [],
      tofu_pins: {},
      next_after: null,
    };
    for (const userId of members) {
      (page.device_keys as JsonObject)[userId] = Object.fromEntries(this.devices.get(userId) ?? []);
      const keys = this.crossSigning.get(userId);
      if (keys?.master) (page.master_keys as JsonObject)[userId] = keys.master;
      if (keys?.self_signing) (page.self_signing_keys as JsonObject)[userId] = keys.self_signing;
      if (keys?.user_signing && userId === caller) (page.user_signing_keys as JsonObject)[userId] = keys.user_signing;
      const pin = this.pins.get(`${caller}|${userId}`);
      if (pin) (page.tofu_pins as JsonObject)[userId] = pin;
      (page.signatures as Signature[]).push(
        ...this.signatures.filter((s) => s.target_user_id === userId && (s.signer_user_id === caller || s.signer_user_id === s.target_user_id)),
      );
    }
    return page as JsonObject;
  }

  pendingToDevice(userId: string, deviceId: string): Omit<ToDevice, "recipient_user_id" | "recipient_device_id">[] {
    return this.toDevice
      .filter((m) => m.recipient_user_id === userId && m.recipient_device_id === deviceId)
      .map(({ id, sender_user_id, sender_device_id, type, content }) => ({ id, sender_user_id, sender_device_id, type, content }));
  }

  ack(userId: string, deviceId: string, ids: string[]): void {
    for (let i = this.toDevice.length - 1; i >= 0; i -= 1) {
      const m = this.toDevice[i]!;
      if (m.recipient_user_id === userId && m.recipient_device_id === deviceId && ids.includes(m.id)) this.toDevice.splice(i, 1);
    }
  }

  sendToDevice(sender: string, body: JsonObject): JsonObject {
    for (const [userId, devices] of Object.entries(body.messages as Record<string, Record<string, JsonObject>>)) {
      for (const [deviceId, content] of Object.entries(devices)) {
        if (!this.devices.get(userId)?.has(deviceId)) continue;
        this.toDevice.push({
          id: String(this.nextToDevice++),
          recipient_user_id: userId,
          recipient_device_id: deviceId,
          sender_user_id: sender,
          sender_device_id: body.device_id as string,
          type: body.type as string,
          content,
        });
      }
    }
    return { undelivered: [] };
  }

  /** `POST /api/messages` (humans) and `POST /api/apps/v1/messages` (the bot) in an encrypted channel. */
  postMessage(sender: string, body: JsonObject): JsonObject {
    const channel = this.channels.get(body.channel_id as string);
    if (!channel?.members.has(sender)) throw new HttpError(404, { error: "Channel not found" });
    if (body.content !== "" || !body.ciphertext) throw new HttpError(400, { error: "plaintext", code: "E2EE_PLAINTEXT_FORBIDDEN" });
    // A supplied id is the idempotency key (backend: check_idempotent_existing_message).
    if (this.messages.some((m) => m.id === body.id && m.user_id === sender)) return { id: body.id as string };
    if (Number(body.epoch) !== channel.epoch) throw new HttpError(409, { error: "stale", code: "E2EE_STALE_EPOCH", current_epoch: channel.epoch });
    const message: StoredMessage = {
      id: body.id as string,
      channel_id: body.channel_id as string,
      parent_id: (body.parent_id as string | undefined) ?? null,
      user_id: sender,
      message_kind: (body.kind as string | undefined) ?? "message",
      epoch: Number(body.epoch),
      revision: Number(body.revision ?? 0),
      ciphertext: body.ciphertext as JsonObject,
    };
    this.messages.push(message);
    if (sender !== this.botUserId && channel.botAllowed.has(this.botUserId)) {
      this.botEvents.push({
        id: randomUUID(),
        version: 1,
        type: "message.created",
        actor_id: sender,
        channel_id: message.channel_id,
        data: { ...message, encrypted: true },
      });
    }
    return { id: message.id };
  }
}

/** A member's device that behaves like the web client's crypto worker. */
export class HumanDevice {
  readonly userId = randomUUID();
  readonly deviceId = randomUUID();
  readonly storageKey = new Uint8Array(32).fill(7);
  readonly account: Wasm["DeviceAccount"]["prototype"];
  readonly trust: Wasm["TrustStore"]["prototype"];
  ownKeys: Wasm["OwnKeys"]["prototype"];
  readonly olm: Wasm["OlmSessions"]["prototype"];
  readonly inbound: Wasm["InboundSessionStore"]["prototype"];
  private readonly outbound = new Map<string, { session: Wasm["OutboundSession"]["prototype"]; shared: Set<string> }>();
  private readonly outboundOlm = new Map<string, string>();
  private readonly crypto: Wasm;
  private readonly zoo: FakeZoo;

  constructor(crypto: Wasm, zoo: FakeZoo, options: { signDevice?: boolean } = {}) {
    this.crypto = crypto;
    this.zoo = zoo;
    this.account = new crypto.DeviceAccount(this.userId, this.deviceId);
    this.account.generateOneTimeKeys(10);
    this.account.generateFallbackKey();
    const deviceKeys = this.account.deviceKeys();
    zoo.registerDevice(this.userId, { device_keys: deviceKeys, one_time_keys: this.account.oneTimeKeys(), fallback_keys: this.account.fallbackKeys() });
    this.account.markKeysAsPublished();
    const secrets = crypto.OwnSecrets.generate(this.userId);
    const crossSigning = secrets.crossSigning;
    zoo.uploadCrossSigning(this.userId, crossSigning.uploadJson());
    this.trust = new crypto.TrustStore(this.userId);
    this.trust.trustOwnMasterKey(crossSigning.master);
    this.trust.recordOwnUserSigningKey(crossSigning.userSigning);
    if (options.signDevice !== false) {
      const signature = crossSigning.selfSigning.signDevice(deviceKeys);
      zoo.signatures.push({ ...signature, signer_user_id: this.userId });
    }
    this.ownKeys = secrets.ownKeys();
    this.olm = new crypto.OlmSessions();
    this.inbound = new crypto.InboundSessionStore();
  }

  /** Replace the cross-signing keys (as after a reset on another device). */
  resetCrossSigning(): void {
    const secrets = this.crypto.OwnSecrets.generate(this.userId);
    const crossSigning = secrets.crossSigning;
    const current = this.zoo.crossSigning.get(this.userId)!;
    this.zoo.uploadCrossSigning(this.userId, {
      ...crossSigning.uploadJson(),
      expected_keys: { master: publicKeyOf(current.master) ?? null, self_signing: publicKeyOf(current.self_signing) ?? null, user_signing: publicKeyOf(current.user_signing) ?? null },
    });
    const signature = crossSigning.selfSigning.signDevice(this.account.deviceKeys());
    this.zoo.signatures.push({ ...signature, signer_user_id: this.userId });
    this.ownKeys = secrets.ownKeys();
  }

  /** Evaluate the channel's snapshot, share the session key with verified devices, and post `body`. */
  send(channelId: string, body: string, options: { mentions?: string[]; parentId?: string } = {}): { messageId: string; devices: string[] } {
    const page = this.zoo.recipients(channelId, this.userId);
    const epoch = Number(page.membership_epoch);
    let distribution = this.trust.evaluateWithUserSigning(this.crypto.KeySnapshot.fromJson(page), this.deviceId, this.ownKeys.userSigning);
    if (distribution.new_pins.length > 0) {
      for (const pin of distribution.new_pins) {
        this.zoo.uploadPins(this.userId, [pin.pin]);
        this.trust.confirmTofuPin(pin);
      }
      distribution = this.trust.evaluateWithUserSigning(this.crypto.KeySnapshot.fromJson(page), this.deviceId, this.ownKeys.userSigning);
    }
    let current = this.outbound.get(channelId);
    if (!current || current.session.epoch !== epoch) {
      current = { session: new this.crypto.OutboundSession(channelId, epoch, Date.now()), shared: new Set() };
      this.outbound.set(channelId, current);
    }
    const roomKey = current.session.roomKey();
    const pending = distribution.devices.filter((d) => !current.shared.has(`${d.userId}|${d.deviceId}`));
    const claimed = this.zoo.claim(Object.fromEntries(pending.filter((d) => !this.outboundOlm.has(d.curve25519)).map((d) => [d.userId, { [d.deviceId]: "signed_curve25519" }])));
    const messages: Record<string, Record<string, JsonObject>> = {};
    for (const device of pending) {
      const stored = this.outboundOlm.get(device.curve25519);
      const oneTimeKey = Object.values(claimed[device.userId]?.[device.deviceId] ?? {})[0] as JsonObject | undefined;
      const olm = stored ? this.crypto.OlmSession.fromPickle(stored, this.account, this.storageKey) : this.account.createOutboundOlmSession(device, oneTimeKey!);
      (messages[device.userId] ??= {})[device.deviceId] = this.account.encryptRoomKey(olm, device, roomKey);
      this.outboundOlm.set(device.curve25519, olm.pickle(this.account, this.storageKey));
      current.shared.add(`${device.userId}|${device.deviceId}`);
    }
    this.zoo.sendToDevice(this.userId, { device_id: this.deviceId, type: "m.room.encrypted", messages });
    const messageId = randomUUID();
    const mentions = { user_ids: options.mentions ?? [], channel: false, here: false };
    const ciphertext = current.session.encrypt(this.account, {
      message_id: messageId,
      parent_id: options.parentId ?? null,
      revision: 0,
      content: { body, format: "mfm", kind: "message" },
      mentions,
    });
    this.zoo.postMessage(this.userId, { id: messageId, channel_id: channelId, content: "", kind: "message", epoch, revision: 0, ciphertext, mentions, parent_id: options.parentId ?? null });
    return { messageId, devices: distribution.devices.map((d) => `${d.userId}|${d.deviceId}`) };
  }

  /** Process this device's to-device messages (with the sender's keys from `keys/query`). */
  receiveKeys(): number {
    let stored = 0;
    const pending = this.zoo.pendingToDevice(this.userId, this.deviceId);
    for (const row of pending) {
      const decrypted = this.account.decryptToDevice(this.olm, row.content);
      const sender = decrypted.sender!;
      const listed = (this.zoo.queryKeys(this.userId, [sender]).device_keys as Record<string, Record<string, JsonObject>>)[sender]?.[decrypted.senderDevice!] ?? null;
      const roomKey = decrypted.verifyRoomKey(this.account, listed, null);
      if (this.inbound.addRoomKey(roomKey) !== "kept") stored += 1;
    }
    this.zoo.ack(this.userId, this.deviceId, pending.map((row) => row.id));
    return stored;
  }

  decrypt(message: StoredMessage) {
    const { id, ...row } = message;
    return this.inbound.decrypt({ ...row, message_id: id });
  }
}
