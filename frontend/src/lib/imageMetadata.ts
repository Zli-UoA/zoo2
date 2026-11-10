export type ImageMetadata = {
  width?: number | null;
  height?: number | null;
  blurhash?: string | null;
};

/** Bound remote dimensions before using them for layout. */
export function imageAspectRatio({ width, height }: ImageMetadata, fallback = 4 / 3): number {
  return Number.isInteger(width) && Number.isInteger(height) && width! > 0 && height! > 0
    ? Math.min(10, Math.max(0.1, width! / height!))
    : fallback;
}
