import type { AttachmentInfo } from "@zoo/crypto-wasm";
import { createContext, useContext } from "react";

import type { MessageRow } from "../electric/types";

/**
 * 暗号化チャンネルのメッセージを復号できなかった理由（docs/e2ee.md の「受信時の検証」）。
 *
 * - `keyMissing`：Megolm の受信用セッションがまだ届いていない（`zoo.room_key` 待ち）。
 * - `locked`：この端末の暗号ストアを解錠していない（Secret Storage を開いていない）。
 * - `staleEpoch`：セッションと行の `epoch` が合わない。
 * - `invalid`：チャンネル、行、種別、送信者、リプレイ、版のどれかの照合に失敗して破棄した。
 * - `failed`：上のどれにも当たらない復号の失敗。
 * - `unencrypted`：暗号化チャンネルなのに暗号文がない（削除されていない）行。正しいサーバーは DB の
 *   トリガーでこの行を作れないので、サーバーや同期元が書き換えたものとして本文を出さない。
 */
export type UndecryptableReason =
  | "keyMissing"
  | "locked"
  | "staleEpoch"
  | "invalid"
  | "failed"
  | "unencrypted";

/**
 * 送信者の確かさ。
 *
 * - `verified`：Olm 由来のセッションで、送信端末をクロス署名でたどれた。
 * - `unverified`：Olm 由来だが、送信端末を検証済みにできなかった（「未検証の送信者」）。
 * - `unauthenticated`：鍵バックアップ由来のセッションで復号した（「送信者を確かめられない」）。
 */
export type SenderTrust = "verified" | "unverified" | "unauthenticated";

export type MessageDecryption =
  | {
      status: "decrypted";
      /** 復号した本文（MFM）。 */
      body: string;
      /** 復号したペイロードの `content.kind`。行の `message_kind` との照合は済んでいる。 */
      kind: "message" | "emote";
      sender: SenderTrust;
      /** 復号したペイロードの `attachments`（暗号化した添付。#364）。ないときは空。 */
      attachments?: AttachmentInfo[];
      /**
       * 版の照合の警告。`stale` は、このメッセージの新しい版を見たあとに古い版の行が届いた
       * （本文は見た中で最新の版のまま）。`conflict` は、同じ版で中身の違う行が届いた（本文は先に
       * 見た方のまま）。どちらもなければ省く。
       */
      revisionWarning?: "stale" | "conflict";
      /**
       * この結果を出した行の `revision`。行の新しい版を復号し終えるまでは前の版の結果が残るので、
       * 編集はこの版を基準に送る（行が先に進んでいれば 409 になる）。
       */
      rowRevision?: number;
    }
  | { status: "undecryptable"; reason: UndecryptableReason };

/** Worker が返す、版の照合の結果（`zoo-crypto` の `RevisionStatus`）。 */
export type RevisionCheck = "latest" | "stale" | "conflict";

type DecryptedMessage = Extract<MessageDecryption, { status: "decrypted" }>;

/** メッセージごとに、見た中で最新の版と、その版の復号の結果。 */
export type LatestRevisions = Map<string, { revision: number; decryption: DecryptedMessage }>;

/**
 * 復号した行の版を、そのメッセージで見た中で最新の版と照らし、表示する結果を決める
 * （docs/e2ee.md の「受信時の検証」の版）。
 *
 * - 見た中で最新の版より古い行（Worker が `stale` と返した、または前に復号した古い版の結果を
 *   使い回した）は、最新の版の本文を表示したまま `stale` と警告する。
 * - 同じ版で中身が違う（`conflict`）は、先に見た方の本文を表示したまま警告する。
 * - それ以外は、この行を最新の版として覚えて、そのまま表示する。
 */
export function applyRevision(
  latest: LatestRevisions,
  messageId: string,
  revision: number,
  decryption: MessageDecryption,
  check: RevisionCheck,
): MessageDecryption {
  if (decryption.status !== "decrypted") return decryption;
  const seen = latest.get(messageId);
  if (check === "stale" || (seen && revision < seen.revision)) {
    return { ...(seen?.decryption ?? decryption), revisionWarning: "stale" };
  }
  if (check === "conflict") {
    const first = seen && seen.revision === revision ? seen.decryption : decryption;
    return { ...first, revisionWarning: "conflict" };
  }
  if (!seen || revision >= seen.revision) latest.set(messageId, { revision, decryption });
  return decryption;
}

/** 行が暗号文を持つ（暗号化チャンネルのメッセージで、削除されていない）か。 */
export function isEncryptedMessage(message: Pick<MessageRow, "ciphertext">): boolean {
  return message.ciphertext != null;
}

/**
 * 表示に使う復号の結果を決める。
 *
 * 平文のメッセージは `null`（今までどおり `content` を表示する）。暗号文があるのに復号の結果がまだ
 * ないときは、この端末では読めない状態として `locked` を返す。`content` は空なので、そのまま
 * 表示しない。
 */
export function resolveMessageDecryption(
  message: Pick<MessageRow, "ciphertext">,
  decryption: MessageDecryption | undefined,
): MessageDecryption | null {
  if (!isEncryptedMessage(message)) {
    // 暗号化チャンネルの、暗号文のない行（`MessagePane` が `unencrypted` を渡す）。
    return decryption?.status === "undecryptable" && decryption.reason === "unencrypted"
      ? decryption
      : null;
  }
  return decryption ?? { status: "undecryptable", reason: "locked" };
}

const EMPTY: ReadonlyMap<string, MessageDecryption> = new Map();

/** 表示中のチャンネルの復号の結果（メッセージの ID ごと）。`MessagePane` が渡す。 */
export const MessageDecryptionsContext =
  createContext<ReadonlyMap<string, MessageDecryption>>(EMPTY);

/** `id` のメッセージの復号の結果。なければ `undefined`。 */
export function useMessageDecryption(id: string): MessageDecryption | undefined {
  return useContext(MessageDecryptionsContext).get(id);
}
