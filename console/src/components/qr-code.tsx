import { useMemo } from "react";
import { create } from "qrcode";

/**
 * QR code rendered locally as SVG paths — the otpauth:// secret never leaves
 * the browser (no external QR API, no canvas, no data: URL). Dark modules on a
 * light tile in both themes so every authenticator app can scan it.
 */
export function QrCode({ value, label, size = 184 }: { value: string; label: string; size?: number }) {
  const { path, n } = useMemo(() => {
    const qr = create(value, { errorCorrectionLevel: "M" });
    const count = qr.modules.size;
    let d = "";
    for (let y = 0; y < count; y++) {
      for (let x = 0; x < count; x++) {
        if (qr.modules.get(y, x)) d += `M${x + 4} ${y + 4}h1v1h-1z`;
      }
    }
    return { path: d, n: count + 8 };
  }, [value]);

  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${n} ${n}`}
      width={size}
      height={size}
      shapeRendering="crispEdges"
      className="rounded-md"
    >
      <rect width={n} height={n} fill="#fdfbf8" />
      <path d={path} fill="#221d1a" />
    </svg>
  );
}
