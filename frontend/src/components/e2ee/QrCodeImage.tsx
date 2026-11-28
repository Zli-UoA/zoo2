import { useMemo } from "react";

import { qrModules } from "../../lib/e2ee/qrCode";

/** QR の周りの余白（モジュール数。仕様の推奨は 4）。 */
const QUIET_ZONE = 4;

/**
 * 検証の QR コード（`SasView.qr_code` のバイト列）。読み取りやすいように、明暗のテーマによらず白地に
 * 黒で描く。
 */
export function QrCodeImage({ bytes, label }: { bytes: number[]; label: string }) {
  const { size, path } = useMemo(() => {
    const modules = qrModules(bytes);
    const count = modules.length;
    let d = "";
    modules.forEach((row, y) =>
      row.forEach((dark, x) => {
        if (dark) d += `M${x + QUIET_ZONE} ${y + QUIET_ZONE}h1v1h-1z`;
      }),
    );
    return { size: count + QUIET_ZONE * 2, path: d };
  }, [bytes]);
  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${size} ${size}`}
      shapeRendering="crispEdges"
      className="aspect-square w-full max-w-64 rounded-md bg-white text-black"
    >
      <path d={path} fill="currentColor" />
    </svg>
  );
}
