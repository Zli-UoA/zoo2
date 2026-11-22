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
 * 暗号化チャンネルの表示判定を MessagePane とキャッチアップで共有する。
 * `encrypted=false` が届いたときも、以前から暗号化と覚えているチャンネルや
 * `edm:` の暗号化DMは平文として扱わない。
 */
export function useEncryptedDisplay({
  channelId,
  channelName,
  encrypted,
  hasCiphertext,
}: {
  channelId: string;
  channelName?: string;
  encrypted?: boolean;
  hasCiphertext: boolean;
}) {
  const e2ee = useE2ee();
  if (encrypted) noteEncryptedChannel(channelId);
  useSyncExternalStore(subscribeEncryptedChannels, encryptedChannelsVersion);
  const [knownEncrypted, setKnownEncrypted] = useState<{
    channelId: string;
    known: KnownEncryptedState;
  } | null>(null);
  useEffect(() => {
    const worker = e2ee.worker;
    if (!worker) return;
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
  const { downgraded, checking } = encryptionSendState({
    channelId,
    encrypted: !!encrypted,
    known: !e2ee.worker
      ? "unavailable"
      : knownEncrypted?.channelId === channelId
        ? knownEncrypted.known
        : "pending",
    channelName,
    hasCiphertext,
  });
  return {
    downgraded,
    checkingEncryption: checking,
    treatAsEncrypted: !!encrypted || isEdm || downgraded,
  };
}
