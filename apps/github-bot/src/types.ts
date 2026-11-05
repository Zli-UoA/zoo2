import { createHash } from "node:crypto";
import type { ZooApp, AppEvent } from "../../../sdk/src/index.ts";
export type Json = Record<string, any>;
export type Zoo = Pick<
  ZooApp,
  "request" | "postMessage" | "updateMessage" | "deleteMessage" | "setBlocks"
>;
export type Kind =
  | "issue"
  | "pull"
  | "discussion"
  | "workflow"
  | "deployment"
  | "release"
  | "commit"
  | "branch"
  | "check";
export interface Target {
  repo: string;
  kind: Kind;
  number: string;
}
export interface Thread extends Target {
  channel: string;
  message: string;
  snapshot: Json;
}
export interface Rule {
  repository: string;
  channels: string[];
  installationId: number;
}
export interface Config {
  zooUrl: string;
  rules: Rule[];
  users: Record<string, string>;
  appId: string;
  privateKey: string;
  githubSecret: string;
  zooSecret: string;
  encryptionKey: Buffer;
  database: string;
  port: number;
  host: string;
}
export interface Subscription {
  repo: string;
  channel: string;
  features: string[];
  label?: string;
  branch?: string;
  workflow?: Record<string, string[]>;
  broadcastComments?: boolean;
  broadcastReviews?: boolean;
}
export class BotError extends Error {
  readonly code: string;
  readonly retryable: boolean;
  readonly retryAfter: number;
  constructor(code: string, retryable = false, retryAfter = 0) {
    super(code);
    this.code = code;
    this.retryable = retryable;
    this.retryAfter = retryAfter;
  }
}
export function id(key: string): string {
  const h = createHash("sha256").update(key).digest("hex");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-5${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`;
}
export function repoName(value: string): string {
  if (
    !/^[a-z\d](?:[a-z\d-]{0,38})\/(?:[a-z\d_.-]{1,100}|\*)$/i.test(value) ||
    value.split("/")[1] === "." ||
    value.split("/")[1] === ".."
  )
    throw new BotError("リポジトリは owner/repo で指定してください。");
  return value.toLowerCase();
}
export function matches(pattern: string, repo: string): boolean {
  return pattern === repo || (pattern.endsWith("/*") && repo.startsWith(pattern.slice(0, -1)));
}
export function ruleFor(config: Config, repo: string, channel?: string): Rule | undefined {
  return config.rules.find(
    (r) => matches(r.repository, repo) && (!channel || r.channels.includes(channel)),
  );
}
export function resource(target: Target): string {
  if (target.kind === "deployment") return `https://github.com/${target.repo}/deployments`;
  if (target.kind === "branch")
    return `https://github.com/${target.repo}/tree/${target.number
      .slice(target.number.indexOf(":") + 1)
      .split("/")
      .map(encodeURIComponent)
      .join("/")}`;
  const path =
    target.kind === "pull"
      ? "pull"
      : target.kind === "issue"
        ? "issues"
        : target.kind === "discussion"
          ? "discussions"
          : target.kind === "workflow"
            ? "actions/runs"
            : target.kind === "release"
              ? "releases/tag"
              : "commit";
  return `https://github.com/${target.repo}/${path}/${encodeURIComponent(target.number)}`;
}
// GitHub text must not synthesize Zoo mentions. Only verified account mappings add mentions.
export function text(value: unknown, max = 1500): string {
  return Array.from(
    String(value ?? "")
      .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f]/g, "")
      .replace(/@/g, "＠"),
  )
    .slice(0, max)
    .join("");
}
export function actor(event: AppEvent): string {
  if (!event.actor_id || !event.channel_id)
    throw new BotError("利用者とチャンネルを確認できません。");
  return event.actor_id;
}
