import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";

import { LocaleProvider } from "../i18n/LocaleProvider";
import { bytesToLatin1, qrModules, type BarcodeDetectorLike } from "../../lib/e2ee/qrCode";
import { QrCodeScanner } from "./QrCodeScanner";

const TXN = "6f1c2b3a-4d5e-4f60-8a7b-9c0d1e2f3a4b";

/** 検証の QR のバイト列（Matrix の形）。鍵と秘密には 0x80〜0xFF のバイトも入れる。 */
const QR = Uint8Array.from([
  ...Array.from("MATRIX", (c) => c.charCodeAt(0)),
  0x02,
  0x00,
  0x00,
  TXN.length,
  ...Array.from(TXN, (c) => c.charCodeAt(0)),
  ...Array.from({ length: 64 }, (_, i) => (i * 37 + 0x80) & 0xff),
  ...Array.from({ length: 16 }, (_, i) => 0xff - i),
]);

const timers: ReturnType<typeof setInterval>[] = [];
afterEach(() => {
  for (const timer of timers.splice(0)) clearInterval(timer);
});

/**
 * QR を描いた canvas を、カメラの代わりの映像にする（`canvas.captureStream`）。映像の枠が流れるように、
 * 描き直し続ける。
 */
function cameraShowing(bytes: Uint8Array): (c: MediaStreamConstraints) => Promise<MediaStream> {
  const modules = qrModules(bytes);
  const scale = 6;
  const size = (modules.length + 8) * scale;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext("2d")!;
  const draw = () => {
    context.fillStyle = "#fff";
    context.fillRect(0, 0, size, size);
    context.fillStyle = "#000";
    modules.forEach((row, y) =>
      row.forEach((dark, x) => {
        if (dark) context.fillRect((x + 4) * scale, (y + 4) * scale, scale, scale);
      }),
    );
  };
  draw();
  timers.push(setInterval(draw, 50));
  const stream = canvas.captureStream(20);
  return async () => stream;
}

function renderScanner(props: Parameters<typeof QrCodeScanner>[0]) {
  return render(
    <LocaleProvider>
      <QrCodeScanner {...props} />
    </LocaleProvider>,
  );
}

const noDetector = async () => null;

it("BarcodeDetector がなければ、JS のデコーダーでカメラの映像から QR のバイト列を読む", async () => {
  const onScan = vi.fn(async (_bytes: Uint8Array) => "accepted" as const);
  const getUserMedia = cameraShowing(QR);
  renderScanner({ onScan, getUserMedia, createDetector: noDetector });
  await waitFor(() => expect(onScan).toHaveBeenCalled(), { timeout: 10_000 });
  expect(onScan.mock.calls[0]![0]).toEqual(QR);
  // accepted のあとは読み取りを止める。
  await new Promise((resolve) => setTimeout(resolve, 600));
  expect(onScan).toHaveBeenCalledTimes(1);
});

it("BarcodeDetector の rawValue がバイト列に戻せなければ、同じ画像を JS のデコーダーで読み直す", async () => {
  const onScan = vi.fn(async (_bytes: Uint8Array) => "accepted" as const);
  // UTF-8 として読んだように、0x80 以上のバイトを置換文字にした値を返す。
  const detect = vi.fn(async () => [{ rawValue: new TextDecoder().decode(QR) }]);
  const detector: BarcodeDetectorLike = { detect };
  const createDetector = async () => detector;
  renderScanner({ onScan, getUserMedia: cameraShowing(QR), createDetector });
  await waitFor(() => expect(onScan).toHaveBeenCalled(), { timeout: 10_000 });
  expect(detect).toHaveBeenCalled();
  expect(onScan.mock.calls[0]![0]).toEqual(QR);
});

it("BarcodeDetector の rawValue が Latin-1 として戻せれば、そのバイト列を使う", async () => {
  const onScan = vi.fn(async (_bytes: Uint8Array) => "accepted" as const);
  const detector: BarcodeDetectorLike = { detect: async () => [{ rawValue: bytesToLatin1(QR) }] };
  const createDetector = async () => detector;
  // 映像には QR を出さない（JS のデコーダーでは読めない）。
  const getUserMedia = cameraShowing(new TextEncoder().encode("x"));
  renderScanner({ onScan, getUserMedia, createDetector });
  await waitFor(() => expect(onScan).toHaveBeenCalled(), { timeout: 10_000 });
  expect(onScan.mock.calls[0]![0]).toEqual(QR);
});

it("別の検証の QR（retry）なら、知らせて読み取りを続ける", async () => {
  const onScan = vi.fn(async (_bytes: Uint8Array) => "retry" as const);
  renderScanner({ onScan, getUserMedia: cameraShowing(QR), createDetector: noDetector });
  const hint = document.querySelector("[aria-live]")!;
  const before = hint.textContent;
  await waitFor(() => expect(onScan.mock.calls.length).toBeGreaterThanOrEqual(2), {
    timeout: 10_000,
  });
  expect(document.querySelector("[aria-live]")!.textContent).not.toBe(before);
});

it("カメラを許可されなければ、その旨を出す", async () => {
  const getUserMedia = async () => {
    throw new DOMException("denied", "NotAllowedError");
  };
  renderScanner({ onScan: vi.fn(), getUserMedia, createDetector: noDetector });
  expect(await screen.findByRole("alert")).toBeInTheDocument();
});

it("閉じたらカメラを止める", async () => {
  const stream = await cameraShowing(QR)({});
  const tracks = stream.getTracks();
  const view = renderScanner({
    onScan: vi.fn(async (_bytes: Uint8Array) => "retry" as const),
    getUserMedia: async () => stream,
    createDetector: noDetector,
  });
  await waitFor(() => expect(document.querySelector("video")!.srcObject).toBe(stream));
  view.unmount();
  for (const track of tracks) expect(track.readyState).toBe("ended");
});

it("accepted で読み取りを終えたら、画面に残っていてもカメラを止める", async () => {
  const stream = await cameraShowing(QR)({});
  const onScan = vi.fn(async (_bytes: Uint8Array) => "accepted" as const);
  renderScanner({ onScan, getUserMedia: async () => stream, createDetector: noDetector });
  await waitFor(() => expect(onScan).toHaveBeenCalled(), { timeout: 10_000 });
  await waitFor(() => {
    for (const track of stream.getTracks()) expect(track.readyState).toBe("ended");
  });
});

it("映像を再生できなければ、カメラを止めて起動に失敗したと出す", async () => {
  const play = vi
    .spyOn(HTMLMediaElement.prototype, "play")
    .mockRejectedValue(new DOMException("blocked", "NotAllowedError"));
  try {
    const stream = await cameraShowing(QR)({});
    const onScan = vi.fn(async (_bytes: Uint8Array) => "accepted" as const);
    renderScanner({ onScan, getUserMedia: async () => stream, createDetector: noDetector });
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    for (const track of stream.getTracks()) expect(track.readyState).toBe("ended");
    expect(onScan).not.toHaveBeenCalled();
  } finally {
    play.mockRestore();
  }
});
