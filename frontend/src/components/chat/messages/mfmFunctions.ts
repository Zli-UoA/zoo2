import type { CSSProperties } from "react";
import type { MfmFn } from "mfm-js";

/** Supported upstream MFM functions (plus the renderer's clickable event extension). */
export const MFM_FUNCTIONS = [
  "tada",
  "jelly",
  "twitch",
  "shake",
  "spin",
  "jump",
  "bounce",
  "flip",
  "x2",
  "x3",
  "x4",
  "scale",
  "position",
  "fg",
  "bg",
  "border",
  "font",
  "blur",
  "rainbow",
  "sparkle",
  "rotate",
  "ruby",
  "unixtime",
  "clickable",
] as const;

const durations: Record<string, number> = {
  tada: 1,
  jelly: 1,
  twitch: 0.5,
  shake: 0.5,
  spin: 1.5,
  jump: 0.75,
  bounce: 0.75,
  rainbow: 1,
  sparkle: 1.5,
};
const fonts = ["serif", "monospace", "cursive", "fantasy", "emoji", "math"];
const borders = [
  "hidden",
  "dotted",
  "dashed",
  "solid",
  "double",
  "groove",
  "ridge",
  "inset",
  "outset",
];

function number(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== "string" || !/^[+-]?(?:\d+\.?\d*|\.\d+)$/.test(value)) return fallback;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.max(min, Math.min(max, parsed)) : fallback;
}
function time(value: unknown, fallback: number, delay = false): string {
  if (typeof value !== "string" || !/^[+-]?(?:\d+\.?\d*|\.\d+)(?:ms|s)$/.test(value))
    return `${fallback}s`;
  const seconds = parseFloat(value) / (value.endsWith("ms") ? 1000 : 1);
  return `${Math.max(delay ? -86400 : 0.5, Math.min(86400, seconds))}s`;
}
function color(value: unknown, fallback: string): string {
  return typeof value === "string" &&
    /^(?:[\da-f]{3}|[\da-f]{4}|[\da-f]{6}|[\da-f]{8})$/i.test(value)
    ? `#${value}`
    : fallback;
}

/** Construct typed, validated properties; never interpret arbitrary user CSS. */
export function functionPresentation({ name, args }: MfmFn["props"]): {
  className: string;
  style: CSSProperties;
} {
  const style: CSSProperties = {};
  let className = "mfm-function";
  if (Object.hasOwn(durations, name)) {
    className += ` mfm-effect mfm-${name}`;
    style.animationDuration = time(args.speed, durations[name]!);
    style.animationDelay = time(args.delay, 0, true);
    if (name === "spin") {
      style.animationDirection = args.left ? "reverse" : args.alternate ? "alternate" : "normal";
      if (args.x) style.animationName = "mfm-spin-x";
      else if (args.y) style.animationName = "mfm-spin-y";
    }
  }
  switch (name) {
    case "flip":
      style.transform = `scale(${args.h || !args.v ? -1 : 1}, ${args.v ? -1 : 1})`;
      break;
    case "x2":
    case "x3":
    case "x4":
      style.fontSize = `${name.slice(1)}em`;
      break;
    case "scale":
      style.transform = `scale(${number(args.x, 1, -5, 5)}, ${number(args.y, 1, -5, 5)})`;
      break;
    case "position":
      style.transform = `translate(${number(args.x, 0, -50, 50)}em, ${number(args.y, 0, -50, 50)}em)`;
      break;
    case "rotate":
      style.transform = `rotate(${number(args.deg, 90, -36000, 36000)}deg)`;
      break;
    case "fg":
      style.color = color(args.color, "#f00");
      break;
    case "bg":
      style.backgroundColor = color(args.color, "#f00");
      break;
    case "border": {
      style.borderColor = color(args.color, "var(--brand)");
      style.borderStyle = (
        borders.includes(String(args.style)) ? args.style : "solid"
      ) as CSSProperties["borderStyle"];
      style.borderWidth = number(args.width, 1, 0, 100);
      style.borderRadius = number(args.radius, 0, 0, 1000);
      if (!args.noclip) style.overflow = "clip";
      break;
    }
    case "font":
      style.fontFamily = fonts.find((font) => args[font]) ?? "inherit";
      break;
    case "blur":
      className += " mfm-blur";
      break;
  }
  return { className, style };
}
