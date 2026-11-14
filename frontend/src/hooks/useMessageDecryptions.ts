import { useEffect, useRef, useState } from "react";

import type { FeedMessage } from "../components/chat/messages/messageFormat";
import { useE2ee } from "../lib/e2ee/e2ee-context";
import {
  applyRevision,
  type LatestRevisions,
  type MessageDecryption,
  type RevisionCheck,
} from "../lib/e2ee/messageDecryption";
import type { EncryptedMessage } from "@zoo/crypto-wasm";

function toEncryptedMessage(message: FeedMessage, channelId: string): EncryptedMessage {
  return {
    channel_id: channelId,
    message_id: message.id,
    parent_id: message.parent_id,
    user_id: message.user_id,
    message_kind: message.message_kind ?? "message",
    epoch: Number(message.epoch ?? 0),
    revision: message.revision ?? 0,
    ciphertext: message.ciphertext as unknown as EncryptedMessage["ciphertext"],
  };
}

/**
 * 暗号文の行を見分けるキー。Worker が照合する値（チャンネル、行、親、送信者、種別、epoch、版、
 * 暗号文の送信端末とセッション）をすべて入れる。どれかが変われば（編集や同期元の書き換え）、前の
 * 結果を使わずに復号と照合をやり直す。
 */
function rowKey(message: FeedMessage, channelId: string): string | null {
  if (!message.ciphertext) return null;
  return JSON.stringify(toEncryptedMessage(message, channelId));
}

/**
 * 暗号化チャンネルの行を、暗号ストアの Worker で復号する（docs/e2ee.md の「受信時の検証」）。
 *
 * 結果はメッセージの ID ごとに返す。Worker がない（このタブが暗号ストアを開いていない、まだ登録中）
 * あいだは空で、行は「この端末で解錠していない」と表示される。鍵が届いたら（`roomKeysVersion`）、
 * 「鍵がまだ届いていない」行を復号し直す。復号した平文はメモリーにだけ置き、保存しない。
 */
export function useMessageDecryptions(
  channelId: string,
  messages: readonly FeedMessage[],
): ReadonlyMap<string, MessageDecryption> {
  const { worker, roomKeysVersion } = useE2ee();
  // 行ごとの復号の結果と、Worker の版の照合の結果。
  const cache = useRef(new Map<string, { value: MessageDecryption; check: RevisionCheck }>());
  // メッセージごとに見た最新の版。キャッシュを使い回したときも、古い版の行を最新として出さない。
  // Worker の版の記録と同じく、チャンネルを切り替えても忘れない（Worker が変われば忘れる）。
  const latest = useRef<LatestRevisions>(new Map());
  const [results, setResults] = useState<ReadonlyMap<string, MessageDecryption>>(new Map());
  const retried = useRef(roomKeysVersion);

  useEffect(() => {
    latest.current.clear();
  }, [worker]);

  useEffect(() => {
    cache.current.clear();
    setResults(new Map());
  }, [channelId, worker]);

  useEffect(() => {
    if (!worker) return;
    if (retried.current !== roomKeysVersion) {
      retried.current = roomKeysVersion;
      for (const [key, { value }] of cache.current) {
        if (value.status === "undecryptable" && value.reason === "keyMissing") {
          cache.current.delete(key);
        }
      }
    }
    const pending = messages.filter((message) => {
      const key = rowKey(message, channelId);
      return key !== null && !cache.current.has(key);
    });
    const publish = () => {
      const next = new Map<string, MessageDecryption>();
      for (const message of messages) {
        const key = rowKey(message, channelId);
        const entry = key ? cache.current.get(key) : undefined;
        if (!entry) continue;
        next.set(
          message.id,
          applyRevision(
            latest.current,
            message.id,
            message.revision ?? 0,
            entry.value,
            entry.check,
          ),
        );
      }
      setResults(next);
    };
    if (pending.length === 0) {
      publish();
      return;
    }
    let active = true;
    void worker
      .request({
        op: "megolmDecrypt",
        rows: pending.map((message) => toEncryptedMessage(message, channelId)),
      })
      .then((outcomes) => {
        pending.forEach((message, index) => {
          const outcome = outcomes[index];
          const key = rowKey(message, channelId);
          if (!outcome || !key) return;
          const value: MessageDecryption =
            outcome.status === "decrypted"
              ? {
                  status: "decrypted",
                  body: outcome.body,
                  kind: outcome.kind,
                  sender: outcome.sender,
                }
              : { status: "undecryptable", reason: outcome.reason };
          const check = outcome.status === "decrypted" ? outcome.revision.status : "latest";
          cache.current.set(key, { value, check });
          // 表示しなくても（チャンネルを離れた）、最新の版は覚えておく。
          applyRevision(latest.current, message.id, message.revision ?? 0, value, check);
        });
        if (active) publish();
      })
      .catch(() => {
        // 暗号ストアの失敗（開き直し中など）。次の変更で試し直す。
      });
    return () => {
      active = false;
    };
  }, [channelId, messages, roomKeysVersion, worker]);

  return results;
}
