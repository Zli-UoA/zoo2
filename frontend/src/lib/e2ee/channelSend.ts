import type { JsonObject, Mentions, OutgoingMessage } from "@zoo/crypto-wasm";

import type { ApiClient } from "../api/client";
import type { CryptoWorker } from "./cryptoWorker";
import {
  claimKeys,
  fetchRecipients,
  postTofuPin,
  sendToDevice,
  staleEpoch,
  staleRevision,
} from "./megolmApi";
import type { MessageDecryption } from "./messageDecryption";
import type { RecipientSummary } from "./store/megolm";

/** 409 `E2EE_STALE_EPOCH` を続けて受けたら諦める回数（docs/e2ee.md の送信の手順 5）。 */
export const STALE_EPOCH_ATTEMPTS = 3;

export interface ChannelCryptoDeps {
  worker: CryptoWorker;
  client: ApiClient;
  /** この端末の `device_id`（to-device の送信元）。 */
  deviceId: string;
  signal?: AbortSignal;
  /** 配布先を判定し直したとき（送信画面の「配れない相手」と配布先の数の表示に使う）。 */
  onRecipients?: (channelId: string, summary: RecipientSummary) => void;
  /**
   * 新しい送信用セッションを用意した（その受信用の写しを暗号ストアに保存した）。鍵バックアップに
   * 送るのに使う（`E2eeContextValue.requestKeyBackup`）。
   */
  onSessionCreated?: () => void;
}

/** 送信の要求に入れる、暗号化チャンネルのメッセージ（docs/e2ee.md の「メッセージ」）。 */
export interface EncryptedMessageRequest {
  id: string;
  channel_id: string;
  content: "";
  parent_id?: string;
  kind: "message";
  epoch: number;
  revision: 0;
  ciphertext: JsonObject;
  mentions: Mentions;
}

/** ピンを置いたあとに判定し直す回数の上限。 */
const MAX_PIN_ROUNDS = 2;

/** 送信を止めた（3 回続けて `E2EE_STALE_EPOCH` を受けた）。下書きに戻して利用者に知らせる。 */
export class EncryptedSendError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EncryptedSendError";
  }
}

/**
 * チャンネルごとに知っている最新の `membership_epoch`。Channels Shape は遅れることがあるので、
 * 409 で受けた `current_epoch` やスナップショットの値と大きい方を使う（遅れた値で送信用セッションを
 * 毎回作り直さないため）。
 */
const knownEpochs = new Map<string, bigint>();

function noteEpoch(channelId: string, epoch: bigint): bigint {
  const known = knownEpochs.get(channelId);
  const next = known === undefined || epoch > known ? epoch : known;
  knownEpochs.set(channelId, next);
  return next;
}

/** テスト用：覚えた epoch を忘れる。 */
export function resetKnownEpochs(): void {
  knownEpochs.clear();
  preparing.clear();
}

/**
 * チャンネルごとの送信用セッションの用意と暗号化を直列にする。続けて送ったときに、両方が同時に
 * スナップショットを取り、同じ端末に鍵を 2 回配らないようにする。送信用セッションを捨てる処理
 * （配布の失敗、作り直し）もこの中でだけ行うので、用意したセッションが暗号化の前に捨てられることは
 * ない（送信の要求は並べてよい）。
 */
const preparing = new Map<string, Promise<unknown>>();

function serialized<T>(channelId: string, task: () => Promise<T>): Promise<T> {
  const previous = preparing.get(channelId) ?? Promise.resolve();
  const next = previous.catch(() => undefined).then(task);
  preparing.set(channelId, next);
  void next
    .finally(() => {
      if (preparing.get(channelId) === next) preparing.delete(channelId);
    })
    .catch(() => undefined);
  return next;
}

/**
 * 送信用セッションを用意し、まだ配っていない検証済みの端末に鍵を配る（docs/e2ee.md の
 * 「membership_epoch と送信の順序」の手順 2）。`minEpoch` は 409 で受けた `current_epoch`。
 */
export function ensureOutboundSession(
  deps: ChannelCryptoDeps,
  channelId: string,
  shapeEpoch: string | null | undefined,
  minEpoch: bigint | null = null,
): Promise<void> {
  return serialized(channelId, () => prepareOutboundSession(deps, channelId, shapeEpoch, minEpoch));
}

async function prepareOutboundSession(
  deps: ChannelCryptoDeps,
  channelId: string,
  shapeEpoch: string | null | undefined,
  minEpoch: bigint | null,
): Promise<void> {
  const epoch = noteEpoch(channelId, minEpoch ?? BigInt(shapeEpoch ?? "0"));
  // 1. 配布先の判定をやり直すか（送信用セッションがない、作り直しの条件に当たった、409 を受けた）。
  const refreshRecipients =
    minEpoch !== null ||
    (await deps.worker.request({ op: "megolmNeedsSnapshot", channelId, epoch: Number(epoch) }));
  if (refreshRecipients) await evaluateRecipients(deps, channelId, minEpoch);
  // 2. 判定をやり直さないときも、必ず配り終えていない端末を確かめて配る（前の配布の途中の失敗、
  //    払い出されなかった端末、`undelivered` の端末）。配り終えるまで、本文は暗号化しない。
  await shareWithPendingDevices(deps, channelId, refreshRecipients);
}

/** `recipients` を取り直して配布先を判定し、初めて見た相手の TOFU のピンを置く。 */
async function evaluateRecipients(
  deps: ChannelCryptoDeps,
  channelId: string,
  minEpoch: bigint | null,
): Promise<void> {
  const { worker, client, signal } = deps;
  let pages = await fetchRecipients(client, channelId, minEpoch, signal);
  for (let round = 0; ; round += 1) {
    const summary = await worker.request({ op: "megolmEvaluate", channelId, pages });
    noteEpoch(channelId, BigInt(summary.epoch));
    const confirmed = [];
    const discarded = [];
    for (const pin of summary.newPins) {
      if (await postTofuPin(client, pin, signal)) confirmed.push(pin);
      else discarded.push(pin);
    }
    if (confirmed.length > 0 || discarded.length > 0) {
      await worker.request({ op: "megolmSettlePins", confirmed, discarded });
    }
    // 初めて見た相手は、ピンを置けるまで配布先に入らない。ピンを置けたら、同じスナップショットで
    // 判定し直してその端末を入れる。別の端末のピンとぶつかったら、そのピンを入れたスナップショットを
    // 取り直して判定し直す。
    if ((confirmed.length > 0 || discarded.length > 0) && round < MAX_PIN_ROUNDS) {
      if (discarded.length > 0) pages = await fetchRecipients(client, channelId, minEpoch, signal);
      continue;
    }
    const addedUserIds = await worker.request({
      op: "megolmNoteRecipients",
      channelId,
      userIds: summary.userIds,
    });
    deps.onRecipients?.(channelId, { ...summary, addedUserIds });
    return;
  }
}

/**
 * 送信用セッションを用意し（epoch が違えば作り直す）、まだ配っていない端末に `zoo.room_key` を配る。
 * 配布の途中で失敗したら（`keys/claim` や `to_device` の通信の失敗、5xx）、だれにも配れていないかも
 * しれないセッションで送らないよう、セッションを捨ててから投げ直す（次の送信で作り直して配る）。
 */
async function shareWithPendingDevices(
  deps: ChannelCryptoDeps,
  channelId: string,
  refreshed: boolean,
): Promise<void> {
  const { worker, client, signal } = deps;
  const plan = await worker.request({ op: "megolmStart", channelId });
  if (refreshed) deps.onSessionCreated?.();
  if (plan.pending === 0) return;
  try {
    const claimed =
      Object.keys(plan.claim).length > 0 ? await claimKeys(client, plan.claim, signal) : {};
    const { messages } = await worker.request({ op: "megolmShare", channelId, claimed });
    const undelivered = await sendToDevice(client, deps.deviceId, messages, signal);
    const skipped = new Set(undelivered.map((target) => `${target.userId}|${target.deviceId}`));
    // 払い出されなかった端末と `undelivered` の端末は配った端末に入れないので、次の送信の前にまた配る。
    const delivered = Object.entries(messages).flatMap(([userId, devices]) =>
      Object.keys(devices)
        .filter((deviceId) => !skipped.has(`${userId}|${deviceId}`))
        .map((deviceId) => ({ userId, deviceId })),
    );
    await worker.request({ op: "megolmMarkShared", channelId, devices: delivered });
  } catch (error) {
    await worker.request({ op: "megolmDiscard", channelId }).catch(() => undefined);
    throw error;
  }
}

/**
 * 送る直前に暗号化して `submit` で送る（docs/e2ee.md の送信の手順）。409 `E2EE_STALE_EPOCH` を受けたら、
 * `current_epoch` 以上のスナップショットで新しいセッションを作って配り直し、同じ内容を暗号化し直して
 * 送り直す。3 回続いたら {@link EncryptedSendError}。
 */
async function encryptAndSubmit(
  deps: ChannelCryptoDeps,
  channelId: string,
  shapeEpoch: string | null | undefined,
  message: OutgoingMessage,
  submit: (ciphertext: JsonObject, epoch: number) => Promise<void>,
): Promise<void> {
  let minEpoch: bigint | null = null;
  for (let attempt = 0; attempt < STALE_EPOCH_ATTEMPTS; attempt += 1) {
    // 用意から暗号化までを、同じチャンネルのほかの送信の用意と重ねない。
    const { ciphertext, epoch } = await serialized(channelId, async () => {
      await prepareOutboundSession(deps, channelId, shapeEpoch, minEpoch);
      return deps.worker.request({ op: "megolmEncrypt", channelId, message });
    });
    try {
      await submit(ciphertext, epoch);
      return;
    } catch (error) {
      const current = staleEpoch(error);
      if (current === null) throw error;
      // 古いセッションでは送らない。`current_epoch` 以上のスナップショットで判定し直すと、epoch の
      // 違う送信用セッションは `megolmStart` が捨てて作り直す。ここで捨てないのは、同じチャンネルの
      // ほかの送信がすでに新しい epoch で用意したセッションを、暗号化の前に捨てないため。
      minEpoch = noteEpoch(channelId, current);
    }
  }
  throw new EncryptedSendError("membership_epoch が変わり続けたため、送信を止めました");
}

/** 新しいメッセージを暗号化して `POST /api/messages` で送る。`message_id` は送り直しでも同じ。 */
export async function sendEncryptedMessage(
  deps: ChannelCryptoDeps,
  options: {
    channelId: string;
    shapeEpoch: string | null | undefined;
    messageId: string;
    parentId: string | null;
    body: string;
    mentions: Mentions;
  },
): Promise<void> {
  const { channelId, messageId, parentId, mentions } = options;
  const message: OutgoingMessage = {
    message_id: messageId,
    parent_id: parentId,
    revision: 0,
    content: { body: options.body, format: "mfm", kind: "message" },
    mentions,
  };
  await encryptAndSubmit(
    deps,
    channelId,
    options.shapeEpoch,
    message,
    async (ciphertext, epoch) => {
      const request: EncryptedMessageRequest = {
        id: messageId,
        channel_id: channelId,
        content: "",
        kind: "message",
        epoch,
        revision: 0,
        ciphertext,
        mentions,
        ...(parentId ? { parent_id: parentId } : {}),
      };
      await deps.client.post("/api/messages", request, { signal: deps.signal });
    },
  );
}

/**
 * 暗号化チャンネルのメッセージを編集する。置き換えた本文を、版 `revision`（今の版 + 1）で暗号化して
 * `PATCH /api/messages/{id}` で送る。編集では通知を作らないので `mentions` は平文に入れない
 * （暗号化したペイロードには入れる）。
 */
export async function editEncryptedMessage(
  deps: ChannelCryptoDeps,
  options: {
    channelId: string;
    shapeEpoch: string | null | undefined;
    messageId: string;
    parentId: string | null;
    revision: number;
    body: string;
    kind: "message" | "emote";
    mentions: Mentions;
  },
): Promise<void> {
  const { channelId, messageId, revision } = options;
  const message: OutgoingMessage = {
    message_id: messageId,
    parent_id: options.parentId,
    revision,
    content: { body: options.body, format: "mfm", kind: options.kind },
    mentions: options.mentions,
  };
  await encryptAndSubmit(
    deps,
    channelId,
    options.shapeEpoch,
    message,
    async (ciphertext, epoch) => {
      await deps.client.patch(
        `/api/messages/${encodeURIComponent(messageId)}`,
        { content: "", epoch, revision, ciphertext },
        { signal: deps.signal },
      );
    },
  );
}

/**
 * 編集の 409 `E2EE_STALE_REVISION` が、自分の編集の応答をなくしただけか（docs/e2ee.md の「メッセージ」）。
 * `current_revision` の行を復号した本文が、送ろうとした本文と同じなら、編集は済んでいる。
 * `shown` は、そのメッセージの今の復号の結果（`rowRevision` の行のもの）。
 */
export function editAlreadyApplied(
  error: unknown,
  shown: MessageDecryption | undefined,
  body: string,
): boolean {
  const current = staleRevision(error);
  return (
    current !== null &&
    shown?.status === "decrypted" &&
    !shown.revisionWarning &&
    shown.rowRevision === current &&
    shown.body === body
  );
}
