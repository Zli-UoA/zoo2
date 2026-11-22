/**
 * デスクトップ版の添付の保存。進捗と中止を、添付の ID ごとに覚えておく。
 *
 * 仮想スクロールでメッセージの行が外れても保存を続け、戻ったときに円の進み具合を出せるように、
 * 状態はコンポーネントではなくこのモジュールが持つ（`useSyncExternalStore`）。
 *
 * 保存先を決めるのは Rust 側（`zoo_download_begin`）だけ。ここは本文を読んで
 * `zoo_download_write` に渡し、最後に `zoo_download_finish` で置き換える。途中で失敗したときと
 * 中止したときは `zoo_download_cancel` で `.part` を消す。
 */

import { useSyncExternalStore } from "react";

import { abortable } from "./abortable";
import { AttachmentDownloadError, type AttachmentUrlResolver } from "./api/attachments";
import {
  desktopDetectionFailure,
  desktopDownloadBegin,
  desktopDownloadCancel,
  desktopDownloadFinish,
  desktopDownloadSettings,
  desktopDownloadWrite,
  detectDesktop,
  hasDesktopBridge,
  isMissingCommandError,
  probeDesktopDownloadSettings,
} from "./desktop";

/** 書き込み中の状態。終わったら（成功・中止・失敗とも）状態を持たない。 */
export type DesktopDownloadState =
  | { status: "preparing" }
  | { status: "downloading"; fraction: number | null }
  | { status: "finishing" };

/** 利用者が中止した。呼び出し側はエラーのトーストを出さない。 */
export class DesktopDownloadCancelledError extends Error {
  constructor() {
    super("Download cancelled");
    this.name = "DesktopDownloadCancelledError";
  }
}

/** 本文の読み取りに失敗した。保存 ID を替えて再試行できることを示す内部用の印。 */
class BodyReadError extends Error {
  readonly original: unknown;

  constructor(original: unknown) {
    super("Body read failed");
    this.name = "BodyReadError";
    this.original = original;
  }
}

function raceAbort<T>(signal: AbortSignal, promise: Promise<T>): Promise<T> {
  if (signal.aborted) return Promise.reject(new DesktopDownloadCancelledError());
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => {
      signal.removeEventListener("abort", onAbort);
      reject(new DesktopDownloadCancelledError());
    };
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      },
    );
  });
}

/** 1 回の IPC で送る量。大きすぎると WebView の IPC が詰まる。 */
const CHUNK_SIZE = 256 * 1024;
/** URL を取り直してやり直すまでの待ち時間（`lib/api/attachments.ts` と同じ）。 */
const RETRY_DELAY_MS = 300;

const states = new Map<string, DesktopDownloadState>();
const controllers = new Map<string, AbortController>();
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of listeners) listener();
}

function setState(id: string, state: DesktopDownloadState | undefined): void {
  if (state) {
    states.set(id, state);
  } else {
    states.delete(id);
  }
  notify();
}

/** 円の表示は 1% 刻みなので、変わらないときは再描画しない。 */
function reportProgress(id: string, fraction: number | null): void {
  const current = states.get(id);
  if (current?.status === "downloading") {
    const before = current.fraction === null ? null : Math.round(current.fraction * 100);
    const after = fraction === null ? null : Math.round(fraction * 100);
    if (before === after) return;
  }
  states.set(id, { status: "downloading", fraction });
  notify();
}

export function subscribeDesktopDownloads(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** 添付の ID の保存の状態。保存していなければ `undefined`。 */
export function desktopDownloadState(id: string): DesktopDownloadState | undefined {
  return states.get(id);
}

export function useDesktopDownloadState(id: string): DesktopDownloadState | undefined {
  return useSyncExternalStore(
    subscribeDesktopDownloads,
    () => states.get(id),
    () => undefined,
  );
}

let available: boolean | null = null;
let probe: Promise<boolean> | null = null;

/**
 * この版のアプリが保存のコマンドを持つか。古い版のアプリでは false（ブラウザーの保存に戻す）。
 * 「コマンドが無い」と分かったときだけ結果を覚え、それ以外の失敗は呼び出し側へ返す
 * （一時的な失敗でブラウザーの保存に切り替えない）。
 */
export async function desktopDownloadsAvailable(): Promise<boolean> {
  if (available !== null) return available;
  if (!probe) {
    probe = (async () => {
      if (!hasDesktopBridge()) {
        available = false;
        return false;
      }
      const desktop = await detectDesktop();
      if (!desktop) {
        // セッションを確かめられなかっただけなら、保存の可否はまだ分からない。
        // ブラウザーの保存に切り替えず、一時的な失敗として呼び出し側に返す。
        const failure = desktopDetectionFailure();
        if (failure) throw failure;
        // プロトコルが合わない古い版のアプリ。保存のコマンドも無い。
        available = false;
        return false;
      }
      try {
        await probeDesktopDownloadSettings();
        available = true;
        return true;
      } catch (error) {
        if (isMissingCommandError(error)) {
          available = false;
          return false;
        }
        throw error;
      }
    })();
  }
  const running = probe;
  try {
    return await running;
  } finally {
    if (probe === running) probe = null;
  }
}

/** 保存を中止する。保存ダイアログを出している間は、決まったあとに取り消す。 */
export function cancelDesktopDownload(id: string): void {
  // 名前への置き換えが始まったら、途中までのファイルはもう無い。押しても保存は止まらないので、
  // ここでは何もしない（ボタン側も無効にする）。
  if (states.get(id)?.status === "finishing") return;
  controllers.get(id)?.abort();
}

type ProgressReporter = (fraction: number | null) => void;

type Runner = (downloadId: string, signal: AbortSignal, report: ProgressReporter) => Promise<void>;

/** 本文の転送が失敗したとき、途中ファイルを捨てて新しい ID でやり直す回数。 */
const BODY_RETRIES = 1;

async function run(
  id: string,
  fileName: string,
  size: number | null | undefined,
  runner: Runner,
  retryDelayMs?: number,
): Promise<void> {
  // 同じ添付を二重に保存しない（円を押したときは中止になる）。
  if (states.has(id)) return;
  const controller = new AbortController();
  controllers.set(id, controller);
  setState(id, { status: "preparing" });
  let downloadId: string | null = null;
  const begin = async (): Promise<string> => {
    const started =
      size == null
        ? await desktopDownloadBegin(fileName)
        : await desktopDownloadBegin(fileName, size);
    if (started.status === "cancelled") throw new DesktopDownloadCancelledError();
    return started.id;
  };
  const report: ProgressReporter = (fraction) => reportProgress(id, fraction);
  try {
    downloadId = await begin();
    if (controller.signal.aborted) {
      await desktopDownloadCancel(downloadId).catch(() => {});
      throw new DesktopDownloadCancelledError();
    }
    setState(id, { status: "downloading", fraction: 0 });
    for (let attempt = 0; ; attempt++) {
      try {
        await runner(downloadId, controller.signal, report);
        break;
      } catch (error) {
        // 再試行するのは本文の読み取りが失敗したときだけ。応答を取る段階で失敗した
        // （HTTP エラーなど）なら、URL の再取得は済んでいるのでそのまま呼び出し側へ返す。
        if (
          !(error instanceof BodyReadError) ||
          retryDelayMs === undefined ||
          attempt >= BODY_RETRIES ||
          controller.signal.aborted
        ) {
          throw error instanceof BodyReadError ? error.original : error;
        }
        // 「保存のたびに確認する」がオンなら begin() が毎回保存ダイアログを出す。
        // 選び直しを求めず、本文読取の失敗として返す。
        const settings = await desktopDownloadSettings().catch(() => null);
        if (settings?.ask_every_time !== false) {
          throw error.original;
        }
        await desktopDownloadCancel(downloadId).catch(() => {});
        downloadId = null;
        await delay(retryDelayMs, controller.signal);
        downloadId = await begin();
        if (controller.signal.aborted) {
          await desktopDownloadCancel(downloadId).catch(() => {});
          throw new DesktopDownloadCancelledError();
        }
        setState(id, { status: "downloading", fraction: 0 });
      }
    }
    if (controller.signal.aborted) throw new DesktopDownloadCancelledError();
    setState(id, { status: "finishing" });
    await desktopDownloadFinish(downloadId);
    downloadId = null;
  } catch (error) {
    if (downloadId) {
      // 失敗したときも、途中までのファイルを残さない。
      await desktopDownloadCancel(downloadId).catch(() => {});
    }
    if (controller.signal.aborted && !(error instanceof DesktopDownloadCancelledError)) {
      throw new DesktopDownloadCancelledError();
    }
    throw error;
  } finally {
    controllers.delete(id);
    setState(id, undefined);
  }
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  return abortable(new Promise<void>((resolve) => setTimeout(resolve, ms)), signal);
}

function contentLength(response: Response): number | null {
  const raw = response.headers.get("content-length");
  if (!raw) return null;
  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? value : null;
}

async function writeChunk(downloadId: string, bytes: Uint8Array): Promise<void> {
  for (let offset = 0; offset < bytes.byteLength; offset += CHUNK_SIZE) {
    const end = Math.min(bytes.byteLength, offset + CHUNK_SIZE);
    // `subarray` のまま渡すと、後ろに余分なバイトが付いたバッファーを送ることがある。
    await desktopDownloadWrite(downloadId, bytes.slice(offset, end));
  }
}

/** 本文の取得だけを再試行対象にする。IPC への書き込み失敗はここを通さない。 */
async function readBody<T>(read: () => Promise<T>, signal: AbortSignal): Promise<T> {
  try {
    return await read();
  } catch (error) {
    if (signal.aborted) throw error;
    throw new BodyReadError(error);
  }
}

async function writeResponse(
  downloadId: string,
  response: Response,
  signal: AbortSignal,
  report: ProgressReporter,
): Promise<void> {
  const total = contentLength(response);
  const reader = response.body?.getReader();
  if (!reader) {
    const bytes = new Uint8Array(await readBody(() => response.arrayBuffer(), signal));
    if (signal.aborted) throw new DesktopDownloadCancelledError();
    await writeChunk(downloadId, bytes);
    report(1);
    return;
  }
  let received = 0;
  try {
    for (;;) {
      const { done, value } = await readBody(() => reader.read(), signal);
      if (done) break;
      if (signal.aborted) throw new DesktopDownloadCancelledError();
      await writeChunk(downloadId, value);
      received += value.byteLength;
      report(total ? Math.min(1, received / total) : null);
    }
  } catch (error) {
    // 中止のときも、書き込みが失敗したときも、未読の本文と取得元の接続を残さない。
    void reader.cancel().catch(() => {});
    throw error;
  }
  report(1);
}

/**
 * 署名付き URL から本文を読みながら保存する。開くのが失敗したら URL を取り直し、本文の
 * 読み取りが失敗したら途中ファイルを捨てて新しい保存として、それぞれ 1 回だけやり直す
 * （`lib/api/attachments.ts` のブラウザーの保存と同じ再試行）。
 */
export async function downloadUrlToDesktop(
  id: string,
  fileName: string,
  resolveUrl: AttachmentUrlResolver,
  options: { retryDelayMs?: number; size?: number | null } = {},
): Promise<void> {
  const retryDelayMs = options.retryDelayMs ?? RETRY_DELAY_MS;
  await run(
    id,
    fileName,
    options.size,
    async (downloadId, signal, report) => {
      const open = async (refresh: boolean): Promise<Response> => {
        // URL の取得はプレビューなどと共有しているので取り消さず、この保存の待機だけを
        // 中止する（中止後の一時ファイルと進捗を残さないため）。
        const url = await abortable(resolveUrl({ refresh }), signal);
        let response: Response;
        try {
          response = await fetch(url, { credentials: "omit", signal });
        } catch (error) {
          throw new AttachmentDownloadError("network", { cause: error });
        }
        if (!response.ok) {
          throw new AttachmentDownloadError("http", { status: response.status });
        }
        return response;
      };
      let response: Response;
      try {
        response = await open(false);
      } catch (error) {
        if (signal.aborted) throw error;
        await delay(retryDelayMs, signal);
        if (signal.aborted) throw error;
        response = await open(true);
      }
      await writeResponse(downloadId, response, signal, report);
    },
    retryDelayMs,
  );
}

/**
 * 復号済み（または取得済み）の `Blob` を保存する。E2EE の添付は復号したものを渡す。
 * 取得と復号を全体の 8 割、書き込みを残りの 2 割として円に出す。
 * 中止は保存側の待機だけ即座に終わらせ、復号は継続してキャッシュに入る。
 */
export async function downloadBlobToDesktop(
  id: string,
  fileName: string,
  load: (signal: AbortSignal, onProgress: (fraction: number | null) => void) => Promise<Blob>,
  size?: number | null,
): Promise<void> {
  await run(id, fileName, size, async (downloadId, signal, report) => {
    const blob = await raceAbort(
      signal,
      load(signal, (fraction) =>
        report(fraction === null ? null : Math.min(0.8, fraction * 0.8)),
      ),
    );
    if (signal.aborted) throw new DesktopDownloadCancelledError();
    const stream = blob.stream?.();
    if (stream) {
      const reader = stream.getReader();
      let received = 0;
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          if (signal.aborted) {
            throw new DesktopDownloadCancelledError();
          }
          await writeChunk(downloadId, value);
          received += value.byteLength;
          report(blob.size > 0 ? 0.8 + 0.2 * Math.min(1, received / blob.size) : 1);
        }
      } catch (error) {
        // 中止のときも、書き込みが失敗したときも、未読の Blob を読み続けない。
        void reader.cancel().catch(() => {});
        throw error;
      }
    } else {
      const bytes = new Uint8Array(await blob.arrayBuffer());
      await writeChunk(downloadId, bytes);
    }
    report(1);
  });
}

/** テストと Storybook 用。覚えている状態と、判定の結果を捨てる。 */
export function resetDesktopDownloadsForTest(): void {
  for (const controller of controllers.values()) controller.abort();
  states.clear();
  controllers.clear();
  listeners.clear();
  available = null;
  probe = null;
}
