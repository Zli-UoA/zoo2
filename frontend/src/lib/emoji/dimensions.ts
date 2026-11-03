import type { CSSProperties } from "react";
import type { DisplayEmoji } from "../electric/types";

/** Reserve the intrinsic image ratio before fetching/decoding its bytes. */
export function emojiImageStyle(
  emoji: DisplayEmoji | undefined,
  height: string | number,
): CSSProperties {
  const ratio = emoji?.width && emoji.height ? emoji.width / emoji.height : 1;
  return {
    height,
    width: typeof height === "number" ? height * ratio : `calc(${height} * ${ratio})`,
    maxWidth: typeof height === "number" ? height * 8 : "8em",
    objectFit: "contain",
  };
}
