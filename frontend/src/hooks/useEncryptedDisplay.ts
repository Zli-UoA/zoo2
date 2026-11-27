import { useEffect, useState, useSyncExternalStore } from "react";

import { useE2ee, type E2eeContextValue } from "../lib/e2ee/e2ee-context";
import {
  encryptedChannelsVersion,
  encryptionSendState,
  noteEncryptedChannel,
  subscribeEncryptedChannels,
  type KnownEncryptedState,
} from "../lib/e2ee/encryptionGuard";

/**
 * このタブが暗号ストアを開いているところで、まもなく `worker` が使えるようになるか。ほかのタブが開いている、
 * 非対応、オフライン、失敗などは、待っても使えるようになるとは限らないので含めない。
 */
function isOpeningCryptoStore(e2ee: E2eeContextValue): boolean {
  return !e2ee.unprovided && !e2ee.worker && e2ee.notReadyReason?.kind === "starting";
}

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
  holdWhileOpening = false,
}: {
  channelId: string | null;
  channelName?: string;
  encrypted?: boolean;
  hasCiphertext: boolean;
  /**
   * 暗号ストアを開いているところも、確認中（`checkingEncryption`）にする。確認の前に本文を出さない
   * 呼び出し元（メッセージリンクの埋め込み）が指定する。省くと、`worker` がない間は
   * `localStorage` などの記録だけで判定する。
   */
  holdWhileOpening?: boolean;
}) {
  const e2ee = useE2ee();
  if (encrypted && channelId) noteEncryptedChannel(channelId);
  useSyncExternalStore(subscribeEncryptedChannels, encryptedChannelsVersion, () => 0);
  // 暗号ストアの答え。尋ねた `worker` とチャンネルの組にだけ使う。
  const [knownEncrypted, setKnownEncrypted] = useState<{
    worker: NonNullable<E2eeContextValue["worker"]>;
    channelId: string;
    known: KnownEncryptedState;
  } | null>(null);
  useEffect(() => {
    const worker = e2ee.worker;
    if (!worker || !channelId) {
      // 前の答えは捨てる。同じ `worker` が `null` を経て戻ったとき（端末の登録し直し）に、古い
      // 「暗号化ではない」を最初の描画で使わず、確かめ直すまで確認中にする。
      setKnownEncrypted(null);
      return;
    }
    let active = true;
    if (encrypted) {
      void worker.request({ op: "megolmRememberEncrypted", channelId }).catch(() => undefined);
    } else {
      setKnownEncrypted({ worker, channelId, known: "pending" });
      void worker
        .request({ op: "megolmIsKnownEncrypted", channelId })
        .then((known) => {
          if (active) setKnownEncrypted({ worker, channelId, known });
        })
        .catch(() => {
          if (active) setKnownEncrypted({ worker, channelId, known: "failed" });
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
          ? holdWhileOpening && isOpeningCryptoStore(e2ee)
            ? "pending"
            : "unavailable"
          : knownEncrypted?.worker === e2ee.worker && knownEncrypted.channelId === channelId
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
