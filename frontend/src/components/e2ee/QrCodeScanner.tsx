import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CameraOffIcon } from "lucide-react";

import {
  bytesFromDetected,
  createBarcodeDetector,
  decodeQrImage,
  type BarcodeDetectorLike,
} from "../../lib/e2ee/qrCode";
import type { QrScanResult } from "../../lib/e2ee/sasController";
import { Spinner } from "../ui/spinner";

/** 読み取りを試す間隔（ミリ秒）。 */
const SCAN_INTERVAL_MS = 250;
/** JS のデコーダーに渡す画像の長辺（大きいほど遅い）。 */
const MAX_FRAME_SIZE = 720;

export type CameraState =
  | "starting"
  | "scanning"
  /** カメラの API がない（安全なコンテキストでない、対応していないブラウザー）。 */
  | "unsupported"
  /** 利用者かブラウザーがカメラの使用を許可しなかった。 */
  | "denied"
  /** カメラがない。 */
  | "noCamera"
  | "failed";

export interface QrCodeScannerProps {
  /**
   * 読み取ったバイト列。`retry` を返したら（別の検証の QR など）、読み取りを続ける。`accepted` なら
   * 止める。
   */
  onScan: (bytes: Uint8Array) => Promise<QrScanResult>;
  /** テスト用：カメラの代わり（effect が開き直さないように、同じ関数を渡し続ける）。 */
  getUserMedia?: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
  /** テスト用：`BarcodeDetector` の代わり（`null` なら JS のデコーダーだけを使う）。 */
  createDetector?: () => Promise<BarcodeDetectorLike | null>;
  /** Storybook 用：カメラを開かずに、この状態を出す。 */
  initialState?: CameraState;
}

function cameraError(error: unknown): CameraState {
  const name = (error as { name?: unknown } | null)?.name;
  if (name === "NotAllowedError" || name === "SecurityError") return "denied";
  if (name === "NotFoundError" || name === "OverconstrainedError") return "noCamera";
  return "failed";
}

/** ブラウザーのカメラ（effect の依存が変わらないように、同じ関数を使い回す）。 */
const browserGetUserMedia = (constraints: MediaStreamConstraints) =>
  navigator.mediaDevices.getUserMedia(constraints);

/** カメラの API があれば `browserGetUserMedia`（安全なコンテキストでなければ `mediaDevices` がない）。 */
function defaultGetUserMedia(): QrCodeScannerProps["getUserMedia"] {
  const media = typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
  return typeof media?.getUserMedia === "function" ? browserGetUserMedia : undefined;
}

/**
 * カメラで相手の検証の QR を読み取る（docs/e2ee.md の「QR コードによる検証」の Web の実装）。
 * `BarcodeDetector` があれば使い、なければ（Firefox、PC の Safari）JS のデコーダーで読む。
 * `BarcodeDetector` の結果をバイト列に戻せないときも、同じ画像を JS のデコーダーで読み直す。
 */
export function QrCodeScanner({
  onScan,
  getUserMedia = defaultGetUserMedia(),
  createDetector = createBarcodeDetector,
  initialState,
}: QrCodeScannerProps) {
  const { t } = useTranslation();
  const videoRef = useRef<HTMLVideoElement>(null);
  const [state, setState] = useState<CameraState>(
    initialState ?? (getUserMedia ? "starting" : "unsupported"),
  );
  const [retried, setRetried] = useState(false);
  const onScanRef = useRef(onScan);
  onScanRef.current = onScan;

  useEffect(() => {
    if (initialState || !getUserMedia) return;
    let stopped = false;
    let stream: MediaStream | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d", { willReadFrequently: true });

    const readFrame = async (
      video: HTMLVideoElement,
      detector: BarcodeDetectorLike | null,
    ): Promise<Uint8Array | null> => {
      if (!context || video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return null;
      const scale = Math.min(1, MAX_FRAME_SIZE / Math.max(video.videoWidth, video.videoHeight));
      const width = Math.max(1, Math.round(video.videoWidth * scale));
      const height = Math.max(1, Math.round(video.videoHeight * scale));
      canvas.width = width;
      canvas.height = height;
      context.drawImage(video, 0, 0, width, height);
      if (detector) {
        try {
          const results = await detector.detect(canvas);
          if (results.length === 0) return null;
          const bytes = bytesFromDetected(results);
          if (bytes) return bytes;
        } catch {
          // 読めなかった画像は、JS のデコーダーで読み直す。
        }
      }
      const image = context.getImageData(0, 0, width, height);
      return decodeQrImage(image.data, width, height);
    };

    void (async () => {
      try {
        stream = await getUserMedia({
          video: { facingMode: { ideal: "environment" } },
          audio: false,
        });
      } catch (error) {
        if (!stopped) setState(cameraError(error));
        return;
      }
      if (stopped) {
        for (const track of stream.getTracks()) track.stop();
        return;
      }
      const video = videoRef.current;
      if (!video) return;
      video.srcObject = stream;
      await video.play().catch(() => undefined);
      const detector = await createDetector().catch(() => null);
      if (stopped) return;
      setState("scanning");
      const tick = async () => {
        if (stopped) return;
        const bytes = await readFrame(video, detector).catch(() => null);
        if (stopped) return;
        if (bytes) {
          const result = await onScanRef.current(bytes).catch(() => "retry" as const);
          if (stopped || result === "accepted") return;
          setRetried(true);
        }
        timer = setTimeout(() => void tick(), SCAN_INTERVAL_MS);
      };
      void tick();
    })();

    return () => {
      stopped = true;
      if (timer !== null) clearTimeout(timer);
      for (const track of stream?.getTracks() ?? []) track.stop();
    };
  }, [initialState, getUserMedia, createDetector]);

  if (state !== "starting" && state !== "scanning") {
    const message = {
      unsupported: t("e2ee.qr.camera.unsupported"),
      denied: t("e2ee.qr.camera.denied"),
      noCamera: t("e2ee.qr.camera.noCamera"),
      failed: t("e2ee.qr.camera.failed"),
    }[state];
    return (
      <p role="alert" className="flex items-start gap-2 text-sm">
        <CameraOffIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
        {message}
      </p>
    );
  }

  return (
    <div className="flex flex-col items-center gap-2">
      <div className="relative w-full max-w-64 overflow-hidden rounded-md bg-muted">
        <video
          ref={videoRef}
          aria-label={t("e2ee.qr.camera.label")}
          className="aspect-square w-full object-cover"
          muted
          playsInline
        />
        {state === "starting" ? (
          <div className="absolute inset-0 flex items-center justify-center">
            <Spinner label={t("e2ee.qr.camera.starting")} />
          </div>
        ) : null}
      </div>
      <p aria-live="polite" className="text-center text-sm text-muted-foreground">
        {retried ? t("e2ee.qr.otherCode") : t("e2ee.qr.scanHint")}
      </p>
    </div>
  );
}
