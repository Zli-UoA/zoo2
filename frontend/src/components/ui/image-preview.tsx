import { useEffect, useRef, useState, type CSSProperties, type ReactEventHandler } from "react";
import { decode } from "blurhash";
import type { ImageMetadata } from "../../lib/imageMetadata";

/** Shared by Home media and channel attachments, including signed URL loading. */
export function ImagePreview({
  src,
  alt,
  blurhash,
  width,
  height,
  className = "",
  imageClassName = "object-contain",
  style,
  onLoad,
  onError,
}: ImageMetadata & {
  src?: string | null;
  alt: string;
  className?: string;
  imageClassName?: string;
  style?: CSSProperties;
  onLoad?: ReactEventHandler<HTMLImageElement>;
  onError?: ReactEventHandler<HTMLImageElement>;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [loadedSrc, setLoadedSrc] = useState<string | null>(null);
  const loaded = !!src && loadedSrc === src;
  useEffect(() => {
    const context = canvas.current?.getContext("2d");
    if (!context) return;
    context.clearRect(0, 0, 32, 32);
    if (!blurhash || blurhash.length > 166) return;
    try {
      context.putImageData(
        new ImageData(new Uint8ClampedArray(decode(blurhash, 32, 32)), 32, 32),
        0,
        0,
      );
    } catch {
      // Optional metadata from peers or old records must never break the image.
    }
  }, [blurhash, loaded]);
  return (
    <div className={`relative overflow-hidden rounded-lg bg-muted ${className}`} style={style}>
      {!loaded && (
        <canvas
          ref={canvas}
          width={32}
          height={32}
          aria-hidden="true"
          className="absolute inset-0 size-full"
        />
      )}
      {src && (
        <img
          key={src}
          src={src}
          alt={alt}
          width={width ?? undefined}
          height={height ?? undefined}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          className={`relative size-full ${imageClassName} ${loaded ? "opacity-100" : "opacity-0"}`}
          ref={(element) => {
            if (element?.complete && element.naturalWidth > 0 && loadedSrc !== src)
              setLoadedSrc(src);
          }}
          onLoad={(event) => {
            setLoadedSrc(src);
            onLoad?.(event);
          }}
          onError={(event) => {
            setLoadedSrc(src);
            onError?.(event);
          }}
        />
      )}
    </div>
  );
}
