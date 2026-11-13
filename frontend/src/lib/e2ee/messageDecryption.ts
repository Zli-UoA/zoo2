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
 */
export type UndecryptableReason = "keyMissing" | "locked" | "staleEpoch" | "invalid" | "failed";

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
    }
  | { status: "undecryptable"; reason: UndecryptableReason };

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
  if (!isEncryptedMessage(message)) return null;
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
