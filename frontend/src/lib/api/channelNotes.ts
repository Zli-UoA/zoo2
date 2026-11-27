import type { ApiClient } from "./client";
import { apiPath } from "./path";

/** 有効ノート一覧の 1 項目。本文は含まない。 */
export interface ChannelNoteSummary {
  id: string;
  channel_id: string;
  title: string;
  position: number;
  version: number;
  created_user_id: string;
  created_at: string;
  updated_at: string;
  /** 一覧のページ継続カーソルに使う。同じ一覧の項目は同じ値を持つ。 */
  order_token: string;
}

/** ノート本文。 */
export interface ChannelNote {
  id: string;
  channel_id: string;
  title: string;
  content: string;
  position: number;
  version: number;
  created_user_id: string;
  created_at: string;
  updated_at: string;
}

/** 削除済み一覧の 1 項目。本文は含まない。 */
export interface DeletedChannelNoteSummary {
  id: string;
  channel_id: string;
  title: string;
  position: number;
  version: number;
  created_user_id: string;
  created_at: string;
  updated_at: string;
  /** API が返す UTC の `YYYY-MM-DDTHH:MM:SS.mmm`。カーソルにはこの原文を使う。 */
  deleted_at: string;
}

export interface DeletedChannelNote extends ChannelNote {
  deleted_at: string;
}

export const NOTES_PAGE_SIZE = 50;
export const NOTE_TITLE_MAX = 200;
export const NOTE_CONTENT_MAX = 100_000;
export const RESTORE_WINDOW_MS = 24 * 60 * 60 * 1000;

const deletedAtPattern = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}$/;
const edgeWhiteSpacePattern = /^\p{White_Space}+|\p{White_Space}+$/gu;

/**
 * Rust の `str::trim` と同じく Unicode White_Space だけを前後から除く。
 * JS の `trim` は U+0085 を残し U+FEFF（BOM）を除くため、API の検証・保存と
 * 同じ境界にするにはこちらを使う。FEFF は White_Space ではないので、有効な
 * タイトルを空文字に変えない。
 */
export function trimNoteTitleWhiteSpace(value: string): string {
  return value.replace(edgeWhiteSpacePattern, "");
}

export function notesPath(channelId: string): string {
  return apiPath`/api/channels/${channelId}/notes`;
}

export function deletedNotesPath(channelId: string): string {
  return apiPath`/api/channels/${channelId}/notes/deleted`;
}

function pageQuery(after: string | undefined, limit: number): string {
  return `limit=${limit}${after ? `&after=${encodeURIComponent(after)}` : ""}`;
}

/** 前ページ最後の項目から、サーバーの並びをそのまま引き継ぐカーソルを作る。 */
export function activeNotesCursor(
  note: Pick<ChannelNoteSummary, "id" | "position" | "order_token">,
): string {
  return `${note.order_token} ${note.position} ${note.id}`;
}

/** 削除済み一覧のカーソル。`deleted_at` は API の原文（UTC・ミリ秒固定）を保つ。 */
export function deletedNotesCursor(
  note: Pick<DeletedChannelNoteSummary, "id" | "deleted_at">,
): string {
  return `${note.deleted_at} ${note.id}`;
}

/** API の `deleted_at` はタイムゾーン表記のない UTC。末尾に `Z` を補って解釈する。 */
export function parseDeletedAtUtc(value: string): Date {
  if (!deletedAtPattern.test(value)) {
    throw new Error(`Invalid deleted_at: ${value}`);
  }
  const date = new Date(`${value}Z`);
  if (!Number.isFinite(date.getTime())) {
    throw new Error(`Invalid deleted_at: ${value}`);
  }
  return date;
}

/** 復元期限は削除時刻の 24 時間後。 */
export function channelNoteExpiresAt(note: Pick<DeletedChannelNoteSummary, "deleted_at">): Date {
  return new Date(parseDeletedAtUtc(note.deleted_at).getTime() + RESTORE_WINDOW_MS);
}

function pad(value: number, length = 2): string {
  return String(value).padStart(length, "0");
}

/**
 * 復元期限を UTC のまま表示する。端末のタイムゾーンで変えると、API の 24 時間判定と
 * 見た目がずれるため、日時は UTC と明示する。
 */
export function formatRestoreDeadline(deletedAt: string): string {
  const date = new Date(parseDeletedAtUtc(deletedAt).getTime() + RESTORE_WINDOW_MS);
  return `${date.getUTCFullYear()}/${pad(date.getUTCMonth() + 1)}/${pad(date.getUTCDate())} ${pad(
    date.getUTCHours(),
  )}:${pad(date.getUTCMinutes())} UTC`;
}

/** 並べ替えの下書き。範囲外・同位置は元の配列をそのまま返す。 */
export function moveNote<T>(items: T[], fromIndex: number, toIndex: number): T[] {
  if (
    fromIndex < 0 ||
    toIndex < 0 ||
    fromIndex >= items.length ||
    toIndex >= items.length ||
    fromIndex === toIndex
  ) {
    return items;
  }
  const next = [...items];
  const [moved] = next.splice(fromIndex, 1);
  next.splice(toIndex, 0, moved!);
  return next;
}

export type NoteValidationError = "titleRequired" | "titleTooLong" | "contentTooLong" | "nul";

/** 作成・更新前に API の制限を同じ条件で確かめる。 */
export function validateNoteDraft({
  title,
  content,
}: {
  title: string;
  content: string;
}): NoteValidationError | null {
  const trimmed = trimNoteTitleWhiteSpace(title);
  if (!trimmed) return "titleRequired";
  if (Array.from(trimmed).length > NOTE_TITLE_MAX) return "titleTooLong";
  if (Array.from(content).length > NOTE_CONTENT_MAX) return "contentTooLong";
  if (title.includes("\0") || content.includes("\0")) return "nul";
  return null;
}

/** 一覧・見出し用。API が有効としたタイトルを、表示だけで別の値に変えない。 */
export function channelNoteTitle(title: string, fallback: string): string {
  return trimNoteTitleWhiteSpace(title) || fallback;
}

/**
 * 新規作成の結果が確定していない失敗か。通信断・408・5xx は、サーバーが作成を
 * 受理した可能性があるため、自動再送せず一覧を再取得して利用者に知らせる。
 */
export function isUnknownCreateOutcome(error: unknown): boolean {
  if (error instanceof DOMException && error.name === "AbortError") return false;
  if (typeof error === "object" && error !== null && "status" in error) {
    const status = Number((error as { status: unknown }).status);
    return status === 408 || status >= 500;
  }
  return true;
}

export interface ChannelNotesApi {
  list(
    channelId: string,
    options?: { after?: string; limit?: number; signal?: AbortSignal },
  ): Promise<ChannelNoteSummary[]>;
  listDeleted(
    channelId: string,
    options?: { after?: string; limit?: number; signal?: AbortSignal },
  ): Promise<DeletedChannelNoteSummary[]>;
  get(channelId: string, noteId: string, options?: { signal?: AbortSignal }): Promise<ChannelNote>;
  getDeleted(
    channelId: string,
    noteId: string,
    options?: { signal?: AbortSignal },
  ): Promise<DeletedChannelNote>;
  create(channelId: string, input: { title: string; content: string }): Promise<ChannelNote>;
  update(
    channelId: string,
    noteId: string,
    input: { title: string; content: string; version: number },
  ): Promise<ChannelNote>;
  remove(channelId: string, noteId: string, version: number): Promise<void>;
  restore(channelId: string, noteId: string): Promise<ChannelNote>;
  reorder(
    channelId: string,
    input: { expectedNoteIds: string[]; noteIds: string[] },
  ): Promise<void>;
}

/**
 * 既存の Notes REST API のクライアント。一覧は本文を含まない概要だけを返し、
 * 本文は選択されたノートの取得 API で読む。
 */
export function createChannelNotesApi(client: ApiClient): ChannelNotesApi {
  const base = (channelId: string) => notesPath(channelId);
  const noteUrl = (channelId: string, noteId: string) =>
    apiPath`/api/channels/${channelId}/notes/${noteId}`;

  return {
    async list(channelId, options = {}) {
      const path = `${base(channelId)}?${pageQuery(options.after, options.limit ?? NOTES_PAGE_SIZE)}`;
      return (await client.requestWithTx<ChannelNoteSummary[]>(path, { signal: options.signal }))
        .data;
    },
    async listDeleted(channelId, options = {}) {
      const path = `${deletedNotesPath(channelId)}?${pageQuery(
        options.after,
        options.limit ?? NOTES_PAGE_SIZE,
      )}`;
      return (
        await client.requestWithTx<DeletedChannelNoteSummary[]>(path, { signal: options.signal })
      ).data;
    },
    async get(channelId, noteId, options = {}) {
      return (
        await client.requestWithTx<ChannelNote>(noteUrl(channelId, noteId), {
          method: "GET",
          ...(options.signal ? { signal: options.signal } : {}),
        })
      ).data;
    },
    async getDeleted(channelId, noteId, options = {}) {
      return (
        await client.requestWithTx<DeletedChannelNote>(
          apiPath`/api/channels/${channelId}/notes/deleted/${noteId}`,
          {
            method: "GET",
            ...(options.signal ? { signal: options.signal } : {}),
          },
        )
      ).data;
    },
    async create(channelId, input) {
      return (
        await client.requestWithTx<ChannelNote>(base(channelId), { method: "POST", body: input })
      ).data;
    },
    async update(channelId, noteId, input) {
      return (
        await client.requestWithTx<ChannelNote>(noteUrl(channelId, noteId), {
          method: "PATCH",
          body: input,
        })
      ).data;
    },
    async remove(channelId, noteId, version) {
      await client.requestWithTx<{ deleted: true }>(noteUrl(channelId, noteId), {
        method: "DELETE",
        body: { version },
      });
    },
    async restore(channelId, noteId) {
      return (
        await client.requestWithTx<ChannelNote>(
          apiPath`/api/channels/${channelId}/notes/${noteId}/restore`,
          { method: "POST" },
        )
      ).data;
    },
    async reorder(channelId, input) {
      await client.requestWithTx<{ updated: true }>(`${base(channelId)}/order`, {
        method: "PATCH",
        body: { expected_note_ids: input.expectedNoteIds, note_ids: input.noteIds },
      });
    },
  };
}
