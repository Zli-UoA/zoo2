import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useBlocker } from "@tanstack/react-router";
import {
  ArrowLeftIcon,
  ArrowUpDownIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  CopyIcon,
  PencilIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SaveIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react";

import { subscribeAuthSessionEnded } from "../../../lib/auth";
import {
  activeNotesCursor,
  channelNoteTitle,
  createChannelNotesApi,
  deletedNotesCursor,
  formatRestoreDeadline,
  isUnknownCreateOutcome,
  moveNote,
  NOTES_PAGE_SIZE,
  validateNoteDraft,
  type ChannelNote,
  type ChannelNoteSummary,
  type DeletedChannelNote,
  type DeletedChannelNoteSummary,
  type NoteValidationError,
} from "../../../lib/api/channelNotes";
import { createConfiguredApiClient } from "../../../lib/api/client";
import { useCollections } from "../../../lib/electric/collections-context";
import { useMediaQuery } from "../../layout/useMediaQuery";
import { ConfirmActionDialog } from "../../ui/confirm-action-dialog";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Skeleton } from "../../ui/skeleton";
import { Textarea } from "../../ui/textarea";
import { MfmContent } from "../messages/MfmContent";

/** 有効ノートの上限。復元時の 409 と作成時の 409 を案内に使う。 */
const MAX_ACTIVE_NOTES = 500;

type LoadStatus = "loading" | "ready" | "error" | "unavailable";

/**
 * 動的なエラー・案内は `notes.` の下の接尾辞として持ち、描画時に
 * `t(\`notes.${suffix}\`)` の静的接頭辞で引く。x18n の dynamic-key 検査を
 * 弱めず、TypeScript でも実在するキーだけを許可する。
 */
type NoteErrorSuffix =
  | "loadError"
  | "deletedLoadError"
  | "deletedMembersOnly"
  | "error.unavailable"
  | "error.notFound"
  | "error.forbidden"
  | "saveFailed"
  | "limit"
  | "conflict"
  | "notFoundUnconfirmed"
  | "writePermissionLost"
  | "deleteConflict"
  | "reorderConflict"
  | "reorderLoadError";

type NoteNoticeSuffix =
  | "saved"
  | "loadMoreConflict"
  | "createUnknown"
  | "createUnknownChecking"
  | "createUnknownRefreshFailed"
  | "deletedNotice"
  | "deleteConflict"
  | "notFoundUnconfirmed"
  | "restored"
  | "restoreUnavailable"
  | "restoreLimit"
  | "restoreFailed"
  | "reorderNeedTwo"
  | "reorderConflict"
  | "reorderLoadError"
  | "reorderSaved"
  | "readPermissionLost"
  | "writePermissionLost"
  | "copied"
  | "copyFailed";

interface ListState<T> {
  status: LoadStatus;
  items: T[];
  error: NoteErrorSuffix | null;
  hasMore: boolean;
  loadingMore: boolean;
  loadingMoreError: NoteErrorSuffix | null;
}

function emptyList<T>(): ListState<T> {
  return {
    status: "loading",
    items: [],
    error: null,
    hasMore: false,
    loadingMore: false,
    loadingMoreError: null,
  };
}

type Selection = { kind: "active"; id: string } | { kind: "deleted"; id: string } | null;

interface DetailState {
  status: "loading" | "ready" | "error" | "unavailable" | "notFound";
  note: ChannelNote | null;
  deletedNote: DeletedChannelNote | null;
  error: NoteErrorSuffix | null;
}

interface EditorState {
  mode: "new" | "edit";
  title: string;
  content: string;
  baseTitle: string;
  baseContent: string;
  baseVersion: number | null;
  saving: boolean;
  error: NoteErrorSuffix | null;
  validation: NoteValidationError | null;
  /**
   * 作成結果が不明なときの確認状態。`checking` は一覧を取得中、`ready` は確認済みで
   * 明示的な再送ができる、`error` は一覧を取得できず再送を保留している。
   */
  unknownCreate: null | "checking" | "ready" | "error";
  conflictLatest: ChannelNote | null;
  /** 編集中に削除されたと確認できたノート。新規ノートとして保存できる。 */
  deletedSource: ChannelNote | null;
  writeLost: boolean;
}

interface Notice {
  tone: "info" | "error";
  key: NoteNoticeSuffix;
}

interface ReorderState {
  items: ChannelNoteSummary[];
  expectedIds: string[];
  saving: boolean;
  error: NoteErrorSuffix | null;
}

function apiStatus(error: unknown): number | null {
  if (typeof error === "object" && error !== null && "status" in error) {
    const status = Number((error as { status: unknown }).status);
    return Number.isFinite(status) ? status : null;
  }
  return null;
}

function listErrorKey(error: unknown, deleted: boolean): NoteErrorSuffix {
  const status = apiStatus(error);
  if (status === 403 || status === 404) {
    return deleted ? "deletedMembersOnly" : "error.unavailable";
  }
  return deleted ? "deletedLoadError" : "loadError";
}

function detailErrorKey(error: unknown): { status: DetailState["status"]; key: NoteErrorSuffix } {
  const status = apiStatus(error);
  if (status === 403 || status === 404) {
    return { status: status === 404 ? "notFound" : "unavailable", key: "error.unavailable" };
  }
  return { status: "error", key: "loadError" };
}

function isDirty(editor: EditorState | null): boolean {
  return (
    !!editor &&
    (editor.title !== editor.baseTitle ||
      editor.content !== editor.baseContent ||
      editor.deletedSource !== null)
  );
}

function reorderHasChanges(reorder: ReorderState | null): boolean {
  return (
    !!reorder &&
    (reorder.items.length !== reorder.expectedIds.length ||
      reorder.items.some((note, index) => note.id !== reorder.expectedIds[index]))
  );
}

function summaryTitle(t: TFunction, title: string): string {
  return channelNoteTitle(title, t("notes.untitled"));
}

/**
 * チャンネルノートの画面。PC では一覧と本文を並べ、狭い画面では一覧から本文へ
 * 切り替える。REST の一覧は概要だけをページ取得し、本文は選択したノートだけを読む。
 */
export function ChannelNotesScreen({
  channelId,
  currentUserId,
  canRead,
  canWrite,
  isMember,
  isArchived,
  onDirtyChange,
  onBusyChange,
}: {
  channelId: string;
  currentUserId: string | null;
  canRead: boolean;
  canWrite: boolean;
  isMember: boolean;
  isArchived: boolean;
  onDirtyChange?: (dirty: boolean) => void;
  /** 保存・並べ替え保存の進行中。親のタブ移動などを止める。 */
  onBusyChange?: (busy: boolean) => void;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const api = useMemo(() => createChannelNotesApi(createConfiguredApiClient(config)), [config]);
  const wide = useMediaQuery("(min-width: 640px)");
  const sessionKey = `${currentUserId ?? ""}:${channelId}`;

  const [active, setActive] = useState<ListState<ChannelNoteSummary>>(emptyList);
  const [deleted, setDeleted] = useState<ListState<DeletedChannelNoteSummary>>(emptyList);
  const [deletedOpen, setDeletedOpen] = useState(false);
  const [selection, setSelection] = useState<Selection>(null);
  const [detail, setDetail] = useState<DetailState | null>(null);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [reorder, setReorder] = useState<ReorderState | null>(null);
  const [reorderLoading, setReorderLoading] = useState(false);
  const [checkingListWithDraft, setCheckingListWithDraft] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<NoteErrorSuffix | null>(null);
  const [restoringId, setRestoringId] = useState<string | null>(null);

  const activeRef = useRef(active);
  activeRef.current = active;
  const deletedRef = useRef(deleted);
  deletedRef.current = deleted;
  const selectionRef = useRef(selection);
  selectionRef.current = selection;
  const editorRef = useRef(editor);
  editorRef.current = editor;
  const reorderRef = useRef(reorder);
  reorderRef.current = reorder;
  const canReadRef = useRef(canRead);
  canReadRef.current = canRead;
  const canWriteRef = useRef(canWrite);
  canWriteRef.current = canWrite;
  const deletedOpenRef = useRef(deletedOpen);
  deletedOpenRef.current = deletedOpen;
  const isMemberRef = useRef(isMember);
  isMemberRef.current = isMember;
  const readLostRef = useRef(false);
  const sessionRef = useRef(sessionKey);
  const sessionEpoch = useRef(0);
  const activeRequest = useRef(0);
  const deletedRequest = useRef(0);
  const detailRequest = useRef(0);
  const activeController = useRef<AbortController | null>(null);
  const deletedController = useRef<AbortController | null>(null);
  const detailController = useRef<AbortController | null>(null);
  const reorderController = useRef<AbortController | null>(null);
  const reorderRequest = useRef(0);
  const dragIndex = useRef<number | null>(null);
  const saveInFlight = useRef(false);
  const deleteInFlight = useRef(false);
  const restoreInFlight = useRef(false);
  const reorderStartInFlight = useRef(false);
  const reorderSaveInFlight = useRef(false);
  const navigationGuardRef = useRef(false);

  const abortRequests = useCallback(() => {
    activeController.current?.abort();
    deletedController.current?.abort();
    detailController.current?.abort();
    reorderController.current?.abort();
    activeController.current = null;
    deletedController.current = null;
    detailController.current = null;
    reorderController.current = null;
  }, []);

  /**
   * 並べ替え画面を閉じる。進行中の準備・保存・競合後の再取得は要求番号で無効にし、
   * 閉じた後に遅れて届いた応答が画面を開き直さないようにする。
   */
  const closeReorder = useCallback(() => {
    reorderRequest.current += 1;
    reorderController.current?.abort();
    reorderController.current = null;
    setReorder(null);
  }, []);

  const clearDrafts = useCallback(() => {
    setSelection(null);
    setDetail(null);
    setEditor(null);
    setCheckingListWithDraft(false);
    closeReorder();
    setDeleteOpen(false);
    setDeleteError(null);
    setRestoringId(null);
    detailRequest.current += 1;
    detailController.current?.abort();
  }, [closeReorder]);

  const resetSession = useCallback(
    (nextNotice: Notice | null = null) => {
      sessionEpoch.current += 1;
      activeRequest.current += 1;
      deletedRequest.current += 1;
      detailRequest.current += 1;
      navigationGuardRef.current = false;
      saveInFlight.current = false;
      deleteInFlight.current = false;
      restoreInFlight.current = false;
      reorderStartInFlight.current = false;
      reorderSaveInFlight.current = false;
      reorderRequest.current += 1;
      abortRequests();
      setActive(emptyList());
      setDeleted(emptyList());
      setDeletedOpen(false);
      setReorderLoading(false);
      setDeleting(false);
      clearDrafts();
      setNotice(nextNotice);
    },
    [abortRequests, clearDrafts],
  );

  // アカウント・チャンネルが変わったら、進行中の応答を捨てて本文と下書きを消す。
  useEffect(() => {
    if (sessionRef.current === sessionKey) return;
    sessionRef.current = sessionKey;
    resetSession();
  }, [resetSession, sessionKey]);

  useEffect(() => {
    return subscribeAuthSessionEnded((reason) => {
      if (reason === "logout" || reason === "reset") {
        resetSession();
      }
    });
  }, [resetSession]);

  useEffect(() => {
    onDirtyChange?.(isDirty(editor) || reorderHasChanges(reorder));
  }, [editor, onDirtyChange, reorder]);

  useEffect(() => {
    navigationGuardRef.current =
      isDirty(editor) || reorderHasChanges(reorder) || Boolean(editor?.saving);
    onBusyChange?.(Boolean(editor?.saving || reorder?.saving));
  }, [editor, onBusyChange, reorder]);

  useBlocker({
    shouldBlockFn: () => navigationGuardRef.current && !window.confirm(t("notes.leaveUnsaved")),
    enableBeforeUnload: () => navigationGuardRef.current,
  });

  useEffect(() => {
    return () => {
      abortRequests();
    };
  }, [abortRequests]);

  const isCurrent = (epoch: number) => epoch === sessionEpoch.current;

  const loadActive = useCallback(
    async (mode: "reset" | "more", attempt = 0): Promise<void> => {
      if (!canReadRef.current) return;
      const epoch = sessionEpoch.current;
      const request = ++activeRequest.current;
      const controller = new AbortController();
      activeController.current?.abort();
      activeController.current = controller;
      if (mode === "reset") {
        setActive((previous) => ({
          ...previous,
          status: "loading",
          error: null,
          loadingMore: false,
          loadingMoreError: null,
        }));
      } else {
        setActive((previous) => ({ ...previous, loadingMore: true, loadingMoreError: null }));
      }
      const currentItems = activeRef.current.items;
      const after =
        mode === "more" && currentItems.length
          ? activeNotesCursor(currentItems[currentItems.length - 1]!)
          : undefined;
      try {
        const data = await api.list(channelId, { after, signal: controller.signal });
        if (!isCurrent(epoch) || request !== activeRequest.current) return;
        setActive((previous) => ({
          status: "ready",
          items:
            mode === "reset"
              ? data
              : [
                  ...previous.items,
                  ...data.filter((note) => !previous.items.some((old) => old.id === note.id)),
                ],
          error: null,
          hasMore: data.length === NOTES_PAGE_SIZE,
          loadingMore: false,
          loadingMoreError: null,
        }));
        // 作成結果不明の下書きを残したまま一覧を取り直したら、確認完了として再送を許す。
        // 確認中の遷移はここに集約し、作成側は一覧の取得を待たない。
        if (
          mode === "reset" &&
          editorRef.current?.unknownCreate &&
          editorRef.current.unknownCreate !== "ready"
        ) {
          setEditor((previous) =>
            previous?.unknownCreate && previous.unknownCreate !== "ready"
              ? { ...previous, unknownCreate: "ready" }
              : previous,
          );
          setNotice({ tone: "error", key: "createUnknown" });
        }
        return;
      } catch (error) {
        if (!isCurrent(epoch) || request !== activeRequest.current || controller.signal.aborted)
          return;
        if (mode === "more" && apiStatus(error) === 409 && attempt < 2) {
          // 前ページ取得後に並び・構成が変わった。位置カーソルでは続きを特定できないため、
          // 先頭から取り直す（有限回）。古い操作は自動で適用し直さない。
          setNotice({ tone: "info", key: "loadMoreConflict" });
          return loadActive("reset", attempt + 1);
        }
        if (mode === "reset") {
          setActive((previous) => ({
            ...previous,
            status: "error",
            error: listErrorKey(error, false),
            loadingMore: false,
          }));
          // 確認中の一覧取得にも失敗したら、確認中で止めず再試行できる状態にする。
          if (editorRef.current?.unknownCreate === "checking") {
            setEditor((previous) =>
              previous?.unknownCreate === "checking"
                ? { ...previous, unknownCreate: "error" }
                : previous,
            );
            setNotice({ tone: "error", key: "createUnknownRefreshFailed" });
          }
        } else {
          setActive((previous) => ({
            ...previous,
            loadingMore: false,
            loadingMoreError: listErrorKey(error, false),
          }));
        }
        return;
      }
    },
    [api, channelId],
  );

  const loadDeleted = useCallback(
    async (mode: "reset" | "more"): Promise<void> => {
      if (!canReadRef.current || !isMember) return;
      const epoch = sessionEpoch.current;
      const request = ++deletedRequest.current;
      const controller = new AbortController();
      deletedController.current?.abort();
      deletedController.current = controller;
      if (mode === "reset") {
        setDeleted((previous) => ({
          ...previous,
          status: "loading",
          error: null,
          loadingMore: false,
          loadingMoreError: null,
        }));
      } else {
        setDeleted((previous) => ({ ...previous, loadingMore: true, loadingMoreError: null }));
      }
      const currentItems = deletedRef.current.items;
      const after =
        mode === "more" && currentItems.length
          ? deletedNotesCursor(currentItems[currentItems.length - 1]!)
          : undefined;
      try {
        const data = await api.listDeleted(channelId, { after, signal: controller.signal });
        if (!isCurrent(epoch) || request !== deletedRequest.current) return;
        setDeleted((previous) => ({
          status: "ready",
          items:
            mode === "reset"
              ? data
              : [
                  ...previous.items,
                  ...data.filter((note) => !previous.items.some((old) => old.id === note.id)),
                ],
          error: null,
          hasMore: data.length === NOTES_PAGE_SIZE,
          loadingMore: false,
          loadingMoreError: null,
        }));
      } catch (error) {
        if (!isCurrent(epoch) || request !== deletedRequest.current || controller.signal.aborted)
          return;
        if (mode === "reset") {
          setDeleted((previous) => ({
            ...previous,
            status: "error",
            error: listErrorKey(error, true),
            loadingMore: false,
          }));
        } else {
          setDeleted((previous) => ({
            ...previous,
            loadingMore: false,
            loadingMoreError: listErrorKey(error, true),
          }));
        }
      }
    },
    [api, channelId, isMember],
  );

  const loadDetail = useCallback(
    async (kind: "active" | "deleted", id: string, silent = false): Promise<void> => {
      if (!canReadRef.current) return;
      if (kind === "deleted" && !isMemberRef.current) return;
      const epoch = sessionEpoch.current;
      const request = ++detailRequest.current;
      const controller = new AbortController();
      detailController.current?.abort();
      detailController.current = controller;
      if (!silent) {
        setDetail({ status: "loading", note: null, deletedNote: null, error: null });
      }
      try {
        if (kind === "active") {
          const note = await api.get(channelId, id, { signal: controller.signal });
          if (!isCurrent(epoch) || request !== detailRequest.current) return;
          setDetail({ status: "ready", note, deletedNote: null, error: null });
        } else {
          const note = await api.getDeleted(channelId, id, { signal: controller.signal });
          if (!isCurrent(epoch) || request !== detailRequest.current) return;
          setDetail({ status: "ready", note: null, deletedNote: note, error: null });
        }
      } catch (error) {
        if (!isCurrent(epoch) || request !== detailRequest.current || controller.signal.aborted)
          return;
        const classified = detailErrorKey(error);
        setDetail({
          status: classified.status,
          note: null,
          deletedNote: null,
          error: classified.key,
        });
        if (kind === "deleted") {
          void loadDeleted("reset");
        }
      }
    },
    [api, channelId, loadDeleted],
  );

  const refreshCurrentView = useCallback(() => {
    void loadActive("reset");
    if (deletedOpenRef.current && isMemberRef.current) {
      void loadDeleted("reset");
    }
    const selected = selectionRef.current;
    const currentEditor = editorRef.current;
    if (selected && !currentEditor && !(selected.kind === "deleted" && !isMemberRef.current)) {
      void loadDetail(selected.kind, selected.id, true);
    }
  }, [isMember, loadActive, loadDeleted, loadDetail]);

  // 開始時とタブ復帰時に REST から取り直す。編集中の下書きは上書きしない。
  useEffect(() => {
    if (!canRead) return;
    void loadActive("reset");
    return () => {
      abortRequests();
    };
  }, [abortRequests, canRead, loadActive, sessionKey]);

  useEffect(() => {
    if (!canRead) return;
    const onVisibility = () => {
      if (document.visibilityState === "visible" && !reorderRef.current) {
        refreshCurrentView();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, [canRead, refreshCurrentView]);

  useEffect(() => {
    if (deletedOpen && isMember && canRead) {
      void loadDeleted("reset");
    } else {
      deletedRequest.current += 1;
      deletedController.current?.abort();
    }
  }, [canRead, deletedOpen, isMember, loadDeleted]);

  // 公開チャンネルで参加状態だけを失ったときは、閲覧権限が残っていても削除済み
  // 一覧・本文を即座に対象外にする。有効一覧へ戻し、古い deleted GET の応答も
  // 要求識別で捨てる。有効ノートの下書きは閲覧権限が残るため保持する。
  useEffect(() => {
    if (isMember) return;
    setDeletedOpen(false);
    setDeleted(emptyList());
    deletedRequest.current += 1;
    deletedController.current?.abort();
    if (selectionRef.current?.kind === "deleted") {
      setSelection(null);
      setDetail(null);
      detailRequest.current += 1;
      detailController.current?.abort();
    }
  }, [isMember]);

  // 閲覧権限を失ったら本文と下書きを即座に消す。書き込みだけを失った場合は下書きを残し、
  // コピーできる読み取り専用の表示にする（手動の離脱とは区別する）。
  useEffect(() => {
    if (!canRead) {
      if (!readLostRef.current) {
        readLostRef.current = true;
        resetSession({ tone: "error", key: "readPermissionLost" });
      }
      return;
    }
    readLostRef.current = false;
    if (!canWrite) {
      setEditor((previous) =>
        previous && isDirty(previous) ? { ...previous, saving: false, writeLost: true } : previous,
      );
      // 並べ替えの準備中や競合後の再取得中でも、古い応答が閉じた画面を開き直さない
      // ように要求番号を進める。開いていた場合だけ案内を出す。
      reorderRequest.current += 1;
      reorderController.current?.abort();
      reorderController.current = null;
      if (reorderRef.current) {
        setReorder(null);
        setNotice({ tone: "error", key: "writePermissionLost" });
      }
    } else {
      setEditor((previous) =>
        previous?.writeLost ? { ...previous, writeLost: false, error: null } : previous,
      );
    }
  }, [canRead, canWrite, resetSession]);

  const confirmDiscard = useCallback((): boolean => {
    if (saveInFlight.current || editorRef.current?.saving) return false;
    if (!isDirty(editorRef.current) && !reorderHasChanges(reorderRef.current)) return true;
    return window.confirm(t("notes.leaveUnsaved"));
  }, [t]);

  function selectActive(note: ChannelNoteSummary) {
    if (reorderStartInFlight.current || !confirmDiscard()) return;
    closeReorder();
    setCheckingListWithDraft(false);
    setSelection({ kind: "active", id: note.id });
    setEditor(null);
    setNotice(null);
    void loadDetail("active", note.id);
  }

  function selectDeleted(note: DeletedChannelNoteSummary) {
    if (!isMember || reorderStartInFlight.current || !confirmDiscard()) return;
    closeReorder();
    setCheckingListWithDraft(false);
    setSelection({ kind: "deleted", id: note.id });
    setEditor(null);
    setNotice(null);
    void loadDetail("deleted", note.id);
  }

  function startNew() {
    if (!canWrite || reorderStartInFlight.current) return;
    if (!confirmDiscard()) return;
    closeReorder();
    setCheckingListWithDraft(false);
    setSelection(null);
    setDetail(null);
    detailRequest.current += 1;
    detailController.current?.abort();
    setNotice(null);
    setEditor({
      mode: "new",
      title: "",
      content: "",
      baseTitle: "",
      baseContent: "",
      baseVersion: null,
      saving: false,
      error: null,
      validation: null,
      unknownCreate: null,
      conflictLatest: null,
      deletedSource: null,
      writeLost: false,
    });
  }

  function startEdit() {
    const note = detail?.note;
    if (!canWrite || !note || reorderStartInFlight.current) return;
    if (!confirmDiscard()) return;
    closeReorder();
    setCheckingListWithDraft(false);
    setEditor({
      mode: "edit",
      title: note.title,
      content: note.content,
      baseTitle: note.title,
      baseContent: note.content,
      baseVersion: note.version,
      saving: false,
      error: null,
      validation: null,
      unknownCreate: null,
      conflictLatest: null,
      deletedSource: null,
      writeLost: false,
    });
  }

  function cancelEditor() {
    if (!confirmDiscard()) return;
    setCheckingListWithDraft(false);
    setEditor(null);
  }

  function toggleDeletedOpen() {
    if (!isMember || reorderStartInFlight.current) return;
    if (!confirmDiscard()) return;
    closeReorder();
    setCheckingListWithDraft(false);
    const next = !deletedOpen;
    setDeletedOpen(next);
    setSelection(null);
    setDetail(null);
    setEditor(null);
    detailRequest.current += 1;
    detailController.current?.abort();
    setNotice(null);
  }

  const finishMutation = useCallback(
    (saved: ChannelNote, key: NoteNoticeSuffix = "saved") => {
      setSelection({ kind: "active", id: saved.id });
      setDetail({ status: "ready", note: saved, deletedNote: null, error: null });
      setEditor(null);
      setCheckingListWithDraft(false);
      closeReorder();
      setNotice({ tone: "info", key });
      void loadActive("reset");
    },
    [closeReorder, loadActive],
  );

  async function submitCreate(editorState: EditorState) {
    const epoch = sessionEpoch.current;
    setEditor((previous) =>
      previous ? { ...previous, saving: true, error: null, validation: null } : previous,
    );
    try {
      const saved = await api.create(channelId, {
        title: editorState.title,
        content: editorState.content,
      });
      if (!isCurrent(epoch)) return;
      finishMutation(saved);
    } catch (error) {
      if (!isCurrent(epoch)) return;
      if (apiStatus(error) === 409) {
        setEditor((previous) =>
          previous ? { ...previous, saving: false, error: "limit" } : previous,
        );
        return;
      }
      if (isUnknownCreateOutcome(error)) {
        // 一覧の再取得が終わるまでは、同じ下書きの再送を許さない。状態遷移は
        // loadActive("reset") に任せ、保存のロックはすぐ解放して操作を続けられるようにする。
        setEditor((previous) =>
          previous
            ? { ...previous, saving: false, unknownCreate: "checking", error: null }
            : previous,
        );
        setNotice({ tone: "error", key: "createUnknownChecking" });
        void loadActive("reset");
        return;
      }
      setEditor((previous) =>
        previous ? { ...previous, saving: false, error: "saveFailed" } : previous,
      );
    }
  }

  /** 作成結果不明のまま一覧の取得に失敗したとき、利用者の操作で確認をやり直す。 */
  function retryUnknownCreateList() {
    if (!editorRef.current?.unknownCreate) return;
    setEditor((previous) =>
      previous?.unknownCreate
        ? { ...previous, saving: false, unknownCreate: "checking", error: null }
        : previous,
    );
    setNotice({ tone: "error", key: "createUnknownChecking" });
    void loadActive("reset");
  }

  async function submitUpdate(editorState: EditorState, version: number) {
    if (selectionRef.current?.kind !== "active") return;
    const noteId = selectionRef.current.id;
    const epoch = sessionEpoch.current;
    setEditor((previous) =>
      previous ? { ...previous, saving: true, error: null, validation: null } : previous,
    );
    try {
      const saved = await api.update(channelId, noteId, {
        title: editorState.title,
        content: editorState.content,
        version,
      });
      if (!isCurrent(epoch)) return;
      finishMutation(saved);
    } catch (error) {
      if (!isCurrent(epoch)) return;
      const status = apiStatus(error);
      if (status === 409) {
        try {
          const latest = await api.get(channelId, noteId);
          if (!isCurrent(epoch)) return;
          setEditor((previous) =>
            previous
              ? {
                  ...previous,
                  saving: false,
                  error: "conflict",
                  conflictLatest: latest,
                }
              : previous,
          );
        } catch (readError) {
          if (!isCurrent(epoch)) return;
          setEditor((previous) =>
            previous
              ? {
                  ...previous,
                  saving: false,
                  error: "conflict",
                  conflictLatest: null,
                }
              : previous,
          );
          if (apiStatus(readError) === 404) {
            await confirmDeletedDuringEdit(noteId);
          }
        }
        return;
      }
      if (status === 404) {
        await confirmDeletedDuringEdit(noteId);
        return;
      }
      if (status === 403) {
        setEditor((previous) =>
          previous
            ? {
                ...previous,
                saving: false,
                error: canWriteRef.current ? "error.forbidden" : "writePermissionLost",
                writeLost: !canWriteRef.current,
              }
            : previous,
        );
        void loadActive("reset");
        return;
      }
      setEditor((previous) =>
        previous ? { ...previous, saving: false, error: "saveFailed" } : previous,
      );
      if (status === 400) {
        // アーカイブ済みなど、権限の変更を検出したら一覧を取り直して再検証する。
        void loadActive("reset");
      }
    }
  }

  async function confirmDeletedDuringEdit(noteId: string) {
    const epoch = sessionEpoch.current;
    try {
      const deletedNote = await api.getDeleted(channelId, noteId);
      if (!isCurrent(epoch)) return;
      setEditor((previous) =>
        previous
          ? {
              ...previous,
              saving: false,
              error: canWriteRef.current ? null : "writePermissionLost",
              deletedSource: deletedNote,
              writeLost: !canWriteRef.current,
            }
          : previous,
      );
    } catch (error) {
      if (!isCurrent(epoch)) return;
      const status = apiStatus(error);
      setEditor((previous) =>
        previous
          ? {
              ...previous,
              saving: false,
              error:
                status === 403
                  ? "writePermissionLost"
                  : status === 404
                    ? "notFoundUnconfirmed"
                    : "saveFailed",
              writeLost: status === 403 && !canWriteRef.current,
            }
          : previous,
      );
    }
    void loadActive("reset");
  }

  async function saveEditor() {
    const current = editorRef.current;
    if (!current || current.saving || !canWrite || saveInFlight.current) return;
    // 一覧で作成結果を確認できるまでは、同じ下書きを再送しない。
    if (current.unknownCreate === "checking" || current.unknownCreate === "error") return;
    const validation = validateNoteDraft({ title: current.title, content: current.content });
    if (validation) {
      setEditor((previous) => (previous ? { ...previous, validation } : previous));
      return;
    }
    const epoch = sessionEpoch.current;
    saveInFlight.current = true;
    try {
      if (current.mode === "new" || (current.deletedSource && current.unknownCreate === "ready")) {
        await submitCreate(current);
      } else if (current.baseVersion !== null) {
        await submitUpdate(current, current.baseVersion);
      }
    } finally {
      if (isCurrent(epoch)) saveInFlight.current = false;
    }
  }

  async function saveAsNew() {
    const current = editorRef.current;
    if (!current || current.saving || !canWrite || saveInFlight.current) return;
    if (current.unknownCreate === "checking" || current.unknownCreate === "error") return;
    const validation = validateNoteDraft({ title: current.title, content: current.content });
    if (validation) {
      setEditor((previous) => (previous ? { ...previous, validation } : previous));
      return;
    }
    const epoch = sessionEpoch.current;
    saveInFlight.current = true;
    try {
      await submitCreate(current);
    } finally {
      if (isCurrent(epoch)) saveInFlight.current = false;
    }
  }

  async function overwriteLatest() {
    const current = editorRef.current;
    const latest = current?.conflictLatest;
    if (!current || !latest || current.saving || !canWrite || saveInFlight.current) return;
    // 比較表示を閉じたり PATCH を送る前に、下書きそのものを検証する。
    const validation = validateNoteDraft({ title: current.title, content: current.content });
    if (validation) {
      setEditor((previous) => (previous ? { ...previous, validation } : previous));
      return;
    }
    const epoch = sessionEpoch.current;
    saveInFlight.current = true;
    try {
      setEditor((previous) =>
        previous ? { ...previous, conflictLatest: null, error: null } : previous,
      );
      await submitUpdate({ ...current, saving: true }, latest.version);
    } finally {
      if (isCurrent(epoch)) saveInFlight.current = false;
    }
  }

  function useLatest() {
    const latest = editorRef.current?.conflictLatest;
    if (!latest) return;
    setEditor((previous) =>
      previous
        ? {
            ...previous,
            title: latest.title,
            content: latest.content,
            baseTitle: latest.title,
            baseContent: latest.content,
            baseVersion: latest.version,
            conflictLatest: null,
            error: null,
            validation: null,
            unknownCreate: null,
          }
        : previous,
    );
  }

  async function copyDraft() {
    const current = editorRef.current;
    if (!current) return;
    try {
      await navigator.clipboard.writeText(`${current.title}\n\n${current.content}`);
      setNotice({ tone: "info", key: "copied" });
    } catch {
      setNotice({ tone: "error", key: "copyFailed" });
    }
  }

  async function confirmDelete() {
    const note = detail?.note;
    if (!note || deleting || !canWrite || deleteInFlight.current) return;
    const epoch = sessionEpoch.current;
    deleteInFlight.current = true;
    setDeleting(true);
    setDeleteError(null);
    try {
      await api.remove(channelId, note.id, note.version);
      if (!isCurrent(epoch)) return;
      setDeleteOpen(false);
      setSelection(null);
      setDetail(null);
      setEditor(null);
      setNotice({ tone: "info", key: "deletedNotice" });
      void loadActive("reset");
      if (deletedOpenRef.current) void loadDeleted("reset");
    } catch (error) {
      if (!isCurrent(epoch)) return;
      const status = apiStatus(error);
      if (status === 409) {
        setDeleteOpen(false);
        setNotice({ tone: "error", key: "deleteConflict" });
        await loadDetail("active", note.id, true);
        void loadActive("reset");
      } else if (status === 404) {
        setDeleteOpen(false);
        setNotice({ tone: "error", key: "notFoundUnconfirmed" });
        void loadActive("reset");
        if (deletedOpenRef.current) void loadDeleted("reset");
      } else {
        setDeleteError("saveFailed");
      }
    } finally {
      if (isCurrent(epoch)) {
        deleteInFlight.current = false;
        setDeleting(false);
      }
    }
  }

  async function restore(note: DeletedChannelNoteSummary) {
    if (!canWrite || restoringId || restoreInFlight.current) return;
    const epoch = sessionEpoch.current;
    restoreInFlight.current = true;
    const editorSnapshot = editorRef.current;
    const selectionSnapshot = selectionRef.current;
    const deletedOpenSnapshot = deletedOpenRef.current;
    setRestoringId(note.id);
    setNotice(null);
    const stillOnSameView = () =>
      isCurrent(epoch) &&
      isMemberRef.current &&
      editorRef.current === editorSnapshot &&
      selectionRef.current === selectionSnapshot &&
      deletedOpenRef.current === deletedOpenSnapshot;
    try {
      const saved = await api.restore(channelId, note.id);
      if (!isCurrent(epoch)) return;
      void loadActive("reset");
      void loadDeleted("reset");
      if (!stillOnSameView()) return;
      // 選択が変わっていない復元では、遅れて届く削除済み本文の取得結果を捨てる。
      detailRequest.current += 1;
      detailController.current?.abort();
      setDeletedOpen(false);
      setSelection({ kind: "active", id: saved.id });
      setDetail({ status: "ready", note: saved, deletedNote: null, error: null });
      setEditor(null);
      setNotice({ tone: "info", key: "restored" });
    } catch (error) {
      if (!isCurrent(epoch)) return;
      const status = apiStatus(error);
      if (status === 404) {
        void loadActive("reset");
        void loadDeleted("reset");
        if (!stillOnSameView()) return;
        // 選択を閉じた後に遅れて届く削除済み本文の取得結果を捨てる。
        detailRequest.current += 1;
        detailController.current?.abort();
        setSelection(null);
        setDetail(null);
        setNotice({ tone: "error", key: "restoreUnavailable" });
      } else if (status === 409) {
        setNotice({ tone: "error", key: "restoreLimit" });
        void loadDeleted("reset");
      } else {
        setNotice({ tone: "error", key: "restoreFailed" });
      }
    } finally {
      if (isCurrent(epoch)) {
        restoreInFlight.current = false;
        setRestoringId(null);
      }
    }
  }

  const loadAllActivePages = useCallback(
    async (attempt = 0): Promise<ChannelNoteSummary[]> => {
      const epoch = sessionEpoch.current;
      const controller = new AbortController();
      reorderController.current?.abort();
      reorderController.current = controller;
      const items: ChannelNoteSummary[] = [];
      let after: string | undefined;
      try {
        while (items.length < MAX_ACTIVE_NOTES) {
          const data = await api.list(channelId, {
            after,
            limit: NOTES_PAGE_SIZE,
            signal: controller.signal,
          });
          if (!isCurrent(epoch)) throw new DOMException("Aborted", "AbortError");
          items.push(...data);
          if (data.length < NOTES_PAGE_SIZE || items.length >= MAX_ACTIVE_NOTES) break;
          after = activeNotesCursor(data[data.length - 1]!);
        }
        return items;
      } catch (error) {
        if (apiStatus(error) === 409 && attempt < 2) {
          return loadAllActivePages(attempt + 1);
        }
        throw error;
      }
    },
    [api, channelId],
  );

  async function beginReorder() {
    if (!canWrite || reorderLoading || reorderStartInFlight.current) return;
    if (!confirmDiscard()) return;
    const epoch = sessionEpoch.current;
    const op = ++reorderRequest.current;
    reorderStartInFlight.current = true;
    const editorSnapshot = editorRef.current;
    const selectionSnapshot = selectionRef.current;
    setReorderLoading(true);
    setNotice(null);
    try {
      const items = await loadAllActivePages();
      if (
        !isCurrent(epoch) ||
        op !== reorderRequest.current ||
        !canWriteRef.current ||
        editorRef.current !== editorSnapshot ||
        selectionRef.current !== selectionSnapshot
      ) {
        return;
      }
      if (items.length < 2) {
        setNotice({ tone: "info", key: "reorderNeedTwo" });
        return;
      }
      setReorder({
        items,
        expectedIds: items.map((note) => note.id),
        saving: false,
        error: null,
      });
      setCheckingListWithDraft(false);
      setSelection(null);
      setDetail(null);
      setEditor(null);
      detailRequest.current += 1;
      detailController.current?.abort();
    } catch (error) {
      if (!isCurrent(epoch) || op !== reorderRequest.current || !canWriteRef.current) return;
      if (apiStatus(error) === 409) {
        setNotice({ tone: "error", key: "reorderConflict" });
      } else {
        setNotice({ tone: "error", key: "reorderLoadError" });
      }
    } finally {
      if (isCurrent(epoch)) {
        reorderStartInFlight.current = false;
        setReorderLoading(false);
      }
    }
  }

  function moveReorder(index: number, delta: number) {
    setReorder((previous) => {
      if (!previous || previous.saving) return previous;
      const next = moveNote(previous.items, index, index + delta);
      if (next === previous.items) return previous;
      return { ...previous, items: next, error: null };
    });
  }

  function dropReorder(targetIndex: number, event: DragEvent<HTMLElement>) {
    event.preventDefault();
    const from = dragIndex.current;
    dragIndex.current = null;
    if (from === null) return;
    setReorder((previous) => {
      if (!previous || previous.saving) return previous;
      const next = moveNote(previous.items, from, targetIndex);
      if (next === previous.items) return previous;
      return { ...previous, items: next, error: null };
    });
  }

  async function saveReorder() {
    const current = reorderRef.current;
    if (!current || current.saving || !canWrite || reorderSaveInFlight.current) return;
    const epoch = sessionEpoch.current;
    const op = reorderRequest.current;
    reorderSaveInFlight.current = true;
    setReorder((previous) => (previous ? { ...previous, saving: true, error: null } : previous));
    try {
      await api.reorder(channelId, {
        expectedNoteIds: current.expectedIds,
        noteIds: current.items.map((note) => note.id),
      });
      if (!isCurrent(epoch)) return;
      void loadActive("reset");
      if (op !== reorderRequest.current) return;
      closeReorder();
      setNotice({ tone: "info", key: "reorderSaved" });
    } catch (error) {
      if (!isCurrent(epoch)) return;
      if (apiStatus(error) === 409) {
        // 競合後の再取得中に画面が閉じたり権限を失った場合は、結果を反映しない。
        try {
          const latest = await loadAllActivePages();
          if (!canApplyReorderReload(epoch, op)) return;
          setReorder({
            items: latest,
            expectedIds: latest.map((note) => note.id),
            saving: false,
            error: "reorderConflict",
          });
        } catch {
          if (!canApplyReorderReload(epoch, op)) return;
          setReorder((previous) =>
            previous ? { ...previous, saving: false, error: "reorderLoadError" } : previous,
          );
        }
      } else if (apiStatus(error) === 403 || apiStatus(error) === 400) {
        if (!canApplyReorderReload(epoch, op)) return;
        closeReorder();
        setNotice({ tone: "error", key: "writePermissionLost" });
        void loadActive("reset");
      } else {
        if (!canApplyReorderReload(epoch, op)) return;
        setReorder((previous) =>
          previous ? { ...previous, saving: false, error: "saveFailed" } : previous,
        );
      }
    } finally {
      if (isCurrent(epoch)) reorderSaveInFlight.current = false;
    }
  }

  function canApplyReorderReload(epoch: number, op: number): boolean {
    return (
      isCurrent(epoch) &&
      op === reorderRequest.current &&
      canWriteRef.current &&
      reorderRef.current !== null
    );
  }

  function cancelReorder() {
    if (!confirmDiscard()) return;
    closeReorder();
  }

  const reorderDirty = reorderHasChanges(reorder);
  // 参加状態を失った render では、passive effect の後片付けを待たずに削除済みの
  // 入口・選択・本文を描画対象から外す。狭い画面では有効一覧へ戻る。
  const deletedVisible = isMember && deletedOpen;
  const visibleSelection = !isMember && selection?.kind === "deleted" ? null : selection;
  const visibleDetail =
    !isMember && (selection?.kind === "deleted" || detail?.deletedNote) ? null : detail;

  const listPane = (
    <section
      aria-label={t("notes.title")}
      className="flex min-h-0 w-full flex-col border-r bg-background sm:w-80 sm:shrink-0"
    >
      {!wide && checkingListWithDraft && editor ? (
        <div className="flex shrink-0 flex-col items-start gap-2 border-b bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
          <p>{t("notes.draftPreserved")}</p>
          <Button variant="outline" size="sm" onClick={() => setCheckingListWithDraft(false)}>
            {t("notes.returnToDraft")}
          </Button>
        </div>
      ) : null}
      <div className="flex shrink-0 flex-wrap items-center gap-1 border-b px-2 py-1.5">
        <h2 className="mr-auto px-1 text-sm font-semibold">
          {deletedVisible ? t("notes.deletedTitle") : t("notes.title")}
        </h2>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("notes.refresh")}
          onClick={refreshCurrentView}
          disabled={deletedVisible ? deleted.status === "loading" : active.status === "loading"}
        >
          <RefreshCwIcon />
        </Button>
        {canWrite && !deletedVisible ? (
          <Button
            size="sm"
            variant="outline"
            onClick={startNew}
            disabled={active.status === "loading" || Boolean(editor?.saving) || reorderLoading}
          >
            <PlusIcon />
            {t("notes.new")}
          </Button>
        ) : null}
        {canWrite && !deletedVisible && !reorder ? (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void beginReorder()}
            disabled={
              reorderLoading ||
              active.status !== "ready" ||
              active.items.length === 0 ||
              Boolean(editor?.saving)
            }
          >
            <ArrowUpDownIcon />
            {reorderLoading ? t("notes.loading") : t("notes.reorder")}
          </Button>
        ) : null}
        {isMember ? (
          <Button
            size="sm"
            variant={deletedVisible ? "secondary" : "ghost"}
            onClick={toggleDeletedOpen}
            disabled={Boolean(editor?.saving) || reorderLoading}
          >
            <RotateCcwIcon />
            {deletedVisible ? t("notes.active") : t("notes.deleted")}
          </Button>
        ) : null}
      </div>

      {isArchived ? (
        <p className="border-b bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
          {t("notes.archivedReadOnly")}
        </p>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {reorder ? (
          <ReorderList
            reorder={reorder}
            reorderDirty={reorderDirty}
            onMove={moveReorder}
            onDrop={dropReorder}
            onDragStart={(index) => {
              dragIndex.current = index;
            }}
            onSave={() => void saveReorder()}
            onCancel={cancelReorder}
            canWrite={canWrite}
          />
        ) : deletedVisible ? (
          <DeletedList
            state={deleted}
            canWrite={canWrite && !isArchived}
            restoringId={restoringId}
            disabled={Boolean(editor?.saving) || reorderLoading}
            onSelect={selectDeleted}
            onRestore={(note) => void restore(note)}
            onLoadMore={() => void loadDeleted("more")}
            onRetry={() => void loadDeleted("reset")}
          />
        ) : (
          <ActiveList
            state={active}
            selection={selection}
            disabled={Boolean(editor?.saving) || reorderLoading}
            onSelect={selectActive}
            onLoadMore={() => void loadActive("more")}
            onRetry={() => void loadActive("reset")}
          />
        )}
      </div>
    </section>
  );

  const showContent =
    wide || (!checkingListWithDraft && (Boolean(visibleSelection) || Boolean(editor)));
  const contentPane = (
    <section aria-label={t("notes.contentTitle")} className="flex min-h-0 min-w-0 flex-1 flex-col">
      {!wide ? (
        <div className="flex shrink-0 items-center border-b px-2 py-1.5">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              if (!confirmDiscard()) return;
              closeReorder();
              setCheckingListWithDraft(false);
              setSelection(null);
              setDetail(null);
              setEditor(null);
              detailRequest.current += 1;
              detailController.current?.abort();
            }}
          >
            <ArrowLeftIcon />
            {t("notes.backToList")}
          </Button>
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {editor ? (
          <NoteEditor
            editor={editor}
            canWrite={canWrite && !editor.writeLost}
            wide={wide}
            onChange={(patch) =>
              setEditor((previous) => (previous ? { ...previous, ...patch } : previous))
            }
            onSave={() => void saveEditor()}
            onCancel={cancelEditor}
            onSaveAsNew={() => void saveAsNew()}
            onOverwriteLatest={() => void overwriteLatest()}
            onUseLatest={useLatest}
            onCopy={() => void copyDraft()}
            onRetryUnknownCreateList={retryUnknownCreateList}
            onCheckListWithDraft={() => setCheckingListWithDraft(true)}
          />
        ) : visibleDetail ? (
          <NoteDetailView
            detail={visibleDetail}
            canWrite={canWrite && !isArchived}
            onEdit={startEdit}
            onDelete={() => {
              setDeleteError(null);
              setDeleteOpen(true);
            }}
            onRetry={() => {
              if (selection) void loadDetail(selection.kind, selection.id);
            }}
            onBack={() => {
              setSelection(null);
              setDetail(null);
            }}
          />
        ) : reorder ? (
          <div className="mx-auto max-w-xl p-4 text-sm text-muted-foreground">
            <h2 className="mb-2 text-base font-semibold text-foreground">
              {t("notes.reorderTitle")}
            </h2>
            <p>{t("notes.reorderHelp")}</p>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {t("notes.selectPrompt")}
          </div>
        )}
      </div>
    </section>
  );

  if (!canRead) {
    return (
      <div
        role="alert"
        className="flex flex-1 items-center justify-center p-6 text-sm text-muted-foreground"
      >
        {t("notes.readPermissionLost")}
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {notice ? (
        <div
          role={notice.tone === "error" ? "alert" : "status"}
          className={`flex shrink-0 items-start gap-2 border-b px-3 py-2 text-sm ${
            notice.tone === "error" ? "text-destructive" : "text-muted-foreground"
          }`}
        >
          <span className="flex-1">{t(`notes.${notice.key}`)}</span>
          <button
            type="button"
            className="shrink-0 text-xs underline"
            onClick={() => setNotice(null)}
          >
            {t("common.close")}
          </button>
        </div>
      ) : null}
      <div className="flex min-h-0 flex-1">
        {wide || !showContent ? listPane : null}
        {wide || showContent ? contentPane : null}
      </div>
      <ConfirmActionDialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        title={t("notes.deleteTitle")}
        description={t("notes.deleteDescription", {
          title: detail?.note ? summaryTitle(t, detail.note.title) : "",
        })}
        confirmLabel={deleting ? t("notes.deleting") : t("notes.delete")}
        onConfirm={() => void confirmDelete()}
        pending={deleting}
        error={deleteError ? t(`notes.${deleteError}`) : undefined}
      />
    </div>
  );
}

function NoteListLoadError({ error, onRetry }: { error: NoteErrorSuffix; onRetry: () => void }) {
  const { t } = useTranslation();
  return (
    <div role="alert" className="flex flex-col items-start gap-2 p-4 text-sm">
      <p className="text-destructive">{t(`notes.${error}`)}</p>
      <Button variant="outline" size="sm" onClick={onRetry}>
        {t("notes.retry")}
      </Button>
    </div>
  );
}

function ActiveList({
  state,
  selection,
  disabled,
  onSelect,
  onLoadMore,
  onRetry,
}: {
  state: ListState<ChannelNoteSummary>;
  selection: Selection;
  disabled?: boolean;
  onSelect: (note: ChannelNoteSummary) => void;
  onLoadMore: () => void;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  if (state.status === "loading" && state.items.length === 0) {
    return (
      <div className="flex flex-col gap-2 p-3" aria-label={t("notes.loading")}>
        {[0, 1, 2, 3].map((row) => (
          <Skeleton key={row} className="h-10 w-full rounded-md" />
        ))}
      </div>
    );
  }
  const loadError =
    state.status === "error" ? (
      <NoteListLoadError error={state.error ?? "loadError"} onRetry={onRetry} />
    ) : null;
  if (state.items.length === 0) {
    return loadError ?? <p className="p-4 text-sm text-muted-foreground">{t("notes.empty")}</p>;
  }
  return (
    <div className="flex flex-col">
      {loadError}
      <ul aria-label={t("notes.title")}>
        {state.items.map((note) => (
          <li key={note.id}>
            <button
              type="button"
              disabled={disabled}
              onClick={() => onSelect(note)}
              aria-current={selection?.kind === "active" && selection.id === note.id}
              className="flex w-full flex-col gap-0.5 border-b px-3 py-2 text-left text-sm hover:bg-muted aria-[current=true]:bg-muted"
            >
              <span className="truncate font-medium">{summaryTitle(t, note.title)}</span>
            </button>
          </li>
        ))}
      </ul>
      {state.status !== "ready" ? null : state.loadingMore ? (
        <p role="status" className="p-3 text-center text-sm text-muted-foreground">
          {t("notes.loadingMore")}
        </p>
      ) : state.loadingMoreError ? (
        <div role="alert" className="flex flex-col items-start gap-2 p-3 text-sm">
          <p className="text-destructive">{t(`notes.${state.loadingMoreError}`)}</p>
          <Button variant="outline" size="sm" onClick={onLoadMore}>
            {t("notes.retry")}
          </Button>
        </div>
      ) : state.hasMore ? (
        <Button variant="ghost" size="sm" className="m-2" onClick={onLoadMore}>
          {t("notes.loadMore")}
        </Button>
      ) : null}
    </div>
  );
}

function DeletedList({
  state,
  canWrite,
  restoringId,
  disabled,
  onSelect,
  onRestore,
  onLoadMore,
  onRetry,
}: {
  state: ListState<DeletedChannelNoteSummary>;
  canWrite: boolean;
  restoringId: string | null;
  disabled?: boolean;
  onSelect: (note: DeletedChannelNoteSummary) => void;
  onRestore: (note: DeletedChannelNoteSummary) => void;
  onLoadMore: () => void;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  if (state.status === "loading" && state.items.length === 0) {
    return (
      <div className="flex flex-col gap-2 p-3" aria-label={t("notes.loading")}>
        {[0, 1, 2].map((row) => (
          <Skeleton key={row} className="h-12 w-full rounded-md" />
        ))}
      </div>
    );
  }
  const loadError =
    state.status === "error" ? (
      <NoteListLoadError error={state.error ?? "deletedLoadError"} onRetry={onRetry} />
    ) : null;
  if (state.items.length === 0) {
    return (
      loadError ?? <p className="p-4 text-sm text-muted-foreground">{t("notes.deletedEmpty")}</p>
    );
  }
  return (
    <div className="flex flex-col">
      {loadError}
      <ul aria-label={t("notes.deletedTitle")}>
        {state.items.map((note) => (
          <li key={note.id} className="border-b">
            <button
              type="button"
              disabled={disabled}
              onClick={() => onSelect(note)}
              className="flex w-full flex-col gap-1 px-3 py-2 text-left text-sm hover:bg-muted"
            >
              <span className="truncate font-medium">{summaryTitle(t, note.title)}</span>
              <span className="text-xs text-muted-foreground">
                {t("notes.restoreDeadline", {
                  deadline: formatRestoreDeadline(note.deleted_at),
                })}
              </span>
            </button>
            {canWrite ? (
              <Button
                variant="outline"
                size="sm"
                className="m-2"
                disabled={restoringId !== null || disabled}
                onClick={() => onRestore(note)}
              >
                <RotateCcwIcon />
                {restoringId === note.id ? t("notes.restoring") : t("notes.restore")}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
      {state.status !== "ready" ? null : state.loadingMore ? (
        <p role="status" className="p-3 text-center text-sm text-muted-foreground">
          {t("notes.loadingMore")}
        </p>
      ) : state.loadingMoreError ? (
        <div role="alert" className="flex flex-col items-start gap-2 p-3 text-sm">
          <p className="text-destructive">{t(`notes.${state.loadingMoreError}`)}</p>
          <Button variant="outline" size="sm" onClick={onLoadMore}>
            {t("notes.retry")}
          </Button>
        </div>
      ) : state.hasMore ? (
        <Button variant="ghost" size="sm" className="m-2" onClick={onLoadMore}>
          {t("notes.loadMore")}
        </Button>
      ) : null}
    </div>
  );
}

function NoteDetailView({
  detail,
  canWrite,
  onEdit,
  onDelete,
  onRetry,
  onBack,
}: {
  detail: DetailState;
  canWrite: boolean;
  onEdit: () => void;
  onDelete: () => void;
  onRetry: () => void;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  if (detail.status === "loading") {
    return (
      <div className="flex flex-col gap-3" aria-label={t("notes.loading")}>
        <Skeleton className="h-7 w-1/2" />
        <Skeleton className="h-4 w-full" />
        <Skeleton className="h-4 w-4/5" />
      </div>
    );
  }
  if (detail.status === "notFound") {
    return (
      <div className="flex flex-col items-start gap-2 text-sm">
        <p className="text-muted-foreground">{t("notes.error.notFound")}</p>
        <Button variant="outline" size="sm" onClick={onBack}>
          {t("notes.backToList")}
        </Button>
      </div>
    );
  }
  if (detail.status === "unavailable" || detail.status === "error") {
    return (
      <div className="flex flex-col items-start gap-2 text-sm">
        <p className={detail.status === "error" ? "text-destructive" : "text-muted-foreground"}>
          {t(`notes.${detail.error ?? "loadError"}`)}
        </p>
        <Button variant="outline" size="sm" onClick={onRetry}>
          {t("notes.retry")}
        </Button>
      </div>
    );
  }
  const deleted = detail.deletedNote;
  const note = detail.note ?? deleted;
  if (!note) {
    return <p className="text-sm text-muted-foreground">{t("notes.selectPrompt")}</p>;
  }
  return (
    <article className="mx-auto flex max-w-4xl flex-col gap-4">
      <div className="flex flex-wrap items-start gap-2">
        <h2 className="min-w-0 flex-1 text-xl font-semibold break-words">
          {summaryTitle(t, note.title)}
        </h2>
        {deleted && canWrite ? null : canWrite ? (
          <>
            <Button variant="outline" size="sm" onClick={onEdit}>
              <PencilIcon />
              {t("notes.edit")}
            </Button>
            <Button variant="destructive" size="sm" onClick={onDelete}>
              <Trash2Icon />
              {t("notes.delete")}
            </Button>
          </>
        ) : null}
      </div>
      {deleted ? (
        <p className="rounded border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
          {t("notes.restoreDeadline", { deadline: formatRestoreDeadline(deleted.deleted_at) })}
        </p>
      ) : null}
      <div className="mfm-body text-sm leading-relaxed break-words whitespace-pre-wrap">
        <MfmContent content={note.content} clickable="display" />
      </div>
    </article>
  );
}

function NoteEditor({
  editor,
  canWrite,
  wide,
  onChange,
  onSave,
  onCancel,
  onSaveAsNew,
  onOverwriteLatest,
  onUseLatest,
  onCopy,
  onRetryUnknownCreateList,
  onCheckListWithDraft,
}: {
  editor: EditorState;
  canWrite: boolean;
  wide: boolean;
  onChange: (patch: Partial<EditorState>) => void;
  onSave: () => void;
  onCancel: () => void;
  onSaveAsNew: () => void;
  onOverwriteLatest: () => void;
  onUseLatest: () => void;
  onCopy: () => void;
  onRetryUnknownCreateList: () => void;
  onCheckListWithDraft: () => void;
}) {
  const { t } = useTranslation();
  const readOnly = !canWrite || editor.writeLost || editor.saving;
  const sendBlocked = editor.unknownCreate === "checking" || editor.unknownCreate === "error";
  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-3">
      <h2 className="text-base font-semibold">
        {editor.mode === "new" ? t("notes.new") : t("notes.edit")}
      </h2>
      {editor.writeLost ? (
        <div
          role="alert"
          className="flex flex-col gap-2 rounded border border-destructive/40 p-3 text-sm"
        >
          <p className="text-destructive">{t("notes.writePermissionLost")}</p>
          <Button variant="outline" size="sm" className="self-start" onClick={onCopy}>
            <CopyIcon />
            {t("notes.copyDraft")}
          </Button>
        </div>
      ) : null}
      {editor.unknownCreate ? (
        <div
          role="alert"
          className="flex flex-col gap-2 rounded border border-amber-500/50 bg-amber-500/10 p-3 text-sm"
        >
          <p>
            {editor.unknownCreate === "checking"
              ? t("notes.createUnknownChecking")
              : editor.unknownCreate === "error"
                ? t("notes.createUnknownRefreshFailed")
                : t("notes.duplicateWarning")}
          </p>
          {editor.unknownCreate === "error" ? (
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              onClick={onRetryUnknownCreateList}
            >
              <RefreshCwIcon />
              {t("notes.retry")}
            </Button>
          ) : null}
          {!wide ? (
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              onClick={onCheckListWithDraft}
            >
              {t("notes.checkListWithDraft")}
            </Button>
          ) : null}
        </div>
      ) : null}
      <Input
        aria-label={t("notes.titleLabel")}
        placeholder={t("notes.titlePlaceholder")}
        value={editor.title}
        readOnly={readOnly}
        onChange={(event) => onChange({ title: event.target.value, validation: null })}
      />
      <Textarea
        aria-label={t("notes.contentLabel")}
        placeholder={t("notes.contentPlaceholder")}
        className="min-h-64 resize-y"
        value={editor.content}
        readOnly={readOnly}
        onChange={(event) => onChange({ content: event.target.value, validation: null })}
      />
      {editor.validation ? (
        <p role="alert" className="text-sm text-destructive">
          {t(`notes.validation.${editor.validation}`)}
        </p>
      ) : null}
      <section aria-label={t("notes.preview")} className="rounded border p-3">
        <h3 className="mb-2 text-xs font-semibold text-muted-foreground">{t("notes.preview")}</h3>
        <div className="mfm-body text-sm leading-relaxed break-words whitespace-pre-wrap">
          <MfmContent content={editor.content} clickable="display" />
        </div>
      </section>
      {editor.error ? (
        <p role="alert" className="text-sm text-destructive">
          {t(`notes.${editor.error}`)}
        </p>
      ) : null}
      {editor.deletedSource && canWrite ? (
        <div className="flex flex-col gap-2 rounded border p-3">
          <p className="text-sm">{t("notes.deletedDuringEdit")}</p>
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            onClick={onSaveAsNew}
            disabled={editor.saving || sendBlocked}
          >
            <SaveIcon />
            {t("notes.saveAsNew")}
          </Button>
        </div>
      ) : null}
      {editor.conflictLatest ? (
        <section
          aria-label={t("notes.latestVersion")}
          className="flex flex-col gap-2 rounded border p-3"
        >
          <h3 className="text-sm font-semibold">{t("notes.latestVersion")}</h3>
          <p className="text-sm font-medium break-words">{editor.conflictLatest.title}</p>
          <div className="mfm-body text-sm leading-relaxed break-words whitespace-pre-wrap">
            <MfmContent content={editor.conflictLatest.content} clickable="display" />
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={onUseLatest} disabled={editor.saving}>
              {t("notes.useLatest")}
            </Button>
            <Button size="sm" onClick={onOverwriteLatest} disabled={editor.saving}>
              <SaveIcon />
              {t("notes.overwriteLatest")}
            </Button>
          </div>
        </section>
      ) : null}
      <div className="flex flex-wrap gap-2">
        {canWrite ? (
          <Button onClick={onSave} disabled={editor.saving || sendBlocked}>
            <SaveIcon />
            {editor.saving
              ? t("notes.saving")
              : editor.unknownCreate
                ? t("notes.resend")
                : t("notes.save")}
          </Button>
        ) : null}
        <Button variant="outline" onClick={onCancel} disabled={editor.saving}>
          <XIcon />
          {t("common.cancel")}
        </Button>
      </div>
    </div>
  );
}

function ReorderList({
  reorder,
  reorderDirty,
  canWrite,
  onMove,
  onDrop,
  onDragStart,
  onSave,
  onCancel,
}: {
  reorder: ReorderState;
  reorderDirty: boolean;
  canWrite: boolean;
  onMove: (index: number, delta: number) => void;
  onDrop: (index: number, event: DragEvent<HTMLElement>) => void;
  onDragStart: (index: number) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2 p-2">
      <p className="px-1 text-xs text-muted-foreground">{t("notes.reorderHelp")}</p>
      <ul aria-label={t("notes.reorderTitle")} className="flex flex-col">
        {reorder.items.map((note, index) => (
          <li
            key={note.id}
            draggable={canWrite && !reorder.saving}
            onDragStart={(event) => {
              onDragStart(index);
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", note.id);
            }}
            onDragOver={(event) => {
              if (canWrite && !reorder.saving) event.preventDefault();
            }}
            onDrop={(event) => onDrop(index, event)}
            className="flex items-center gap-1 border-b px-1 py-1"
          >
            <span className="min-w-0 flex-1 truncate text-sm">
              {index + 1}. {summaryTitle(t, note.title)}
            </span>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("notes.reorderUp", { title: summaryTitle(t, note.title) })}
              disabled={!canWrite || reorder.saving || index === 0}
              onClick={() => onMove(index, -1)}
            >
              <ChevronUpIcon />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("notes.reorderDown", { title: summaryTitle(t, note.title) })}
              disabled={!canWrite || reorder.saving || index === reorder.items.length - 1}
              onClick={() => onMove(index, 1)}
            >
              <ChevronDownIcon />
            </Button>
          </li>
        ))}
      </ul>
      {reorder.error ? (
        <p role="alert" className="text-sm text-destructive">
          {t(`notes.${reorder.error}`)}
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button onClick={onSave} disabled={!canWrite || reorder.saving || !reorderDirty}>
          <SaveIcon />
          {reorder.saving ? t("notes.saving") : t("notes.reorderSave")}
        </Button>
        <Button variant="outline" onClick={onCancel} disabled={reorder.saving}>
          {t("notes.reorderCancel")}
        </Button>
      </div>
    </div>
  );
}
