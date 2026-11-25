import { useEffect, useState, useSyncExternalStore } from "react";

import { useE2ee } from "../lib/e2ee/e2ee-context";
import {
  encryptedChannelsVersion,
  encryptionSendState,
  noteEncryptedChannel,
  subscribeEncryptedChannels,
  type KnownEncryptedState,
} from "../lib/e2ee/encryptionGuard";

/**
 * 暗号化チャンネルの表示判定を MessagePane・キャッチアップ・メッセージリンクの埋め込みで共有する。
 * `encrypted=false` が届いたときも、以前から暗号化と覚えているチャンネル（`localStorage` と暗号ストアの記録）や
 * `edm:` の暗号化DM、`ciphertext` のある行を含むチャンネルは平文として扱わない。
 * `channelId` が `null` の間は、判定する対象がないものとして、何も問い合わせない。
 */
export function useEncryptedDisplay({
  channelId,
  channelName,
  encrypted,
  hasCiphertext,
}: {
  channelId: string | null;
  channelName?: string;
  encrypted?: boolean;
  hasCiphertext: boolean;
}) {
  const e2ee = useE2ee();
  if (encrypted && channelId) noteEncryptedChannel(channelId);
  useSyncExternalStore(subscribeEncryptedChannels, encryptedChannelsVersion, () => 0);
  const [knownEncrypted, setKnownEncrypted] = useState<{
    channelId: string;
    known: KnownEncryptedState;
  } | null>(null);
  useEffect(() => {
    const worker = e2ee.worker;
    if (!worker || !channelId) return;
    let active = true;
    if (encrypted) {
      void worker.request({ op: "megolmRememberEncrypted", channelId }).catch(() => undefined);
    } else {
      setKnownEncrypted({ channelId, known: "pending" });
      void worker
        .request({ op: "megolmIsKnownEncrypted", channelId })
        .then((known) => {
          if (active) setKnownEncrypted({ channelId, known });
        })
        .catch(() => {
          if (active) setKnownEncrypted({ channelId, known: "failed" });
        });
    }
    return () => {
      active = false;
    };
  }, [channelId, e2ee.worker, encrypted]);

  const isEdm = !!channelName?.startsWith("edm:");
  const { downgraded, checking } = channelId
    ? encryptionSendState({
        channelId,
        encrypted: !!encrypted,
        known: !e2ee.worker
          ? "unavailable"
          : knownEncrypted?.channelId === channelId
            ? knownEncrypted.known
            : "pending",
        channelName,
        hasCiphertext,
      })
    : { downgraded: false, checking: false };
  return {
    downgraded,
    checkingEncryption: checking,
    treatAsEncrypted: !!channelId && (!!encrypted || isEdm || downgraded),
  };
}
