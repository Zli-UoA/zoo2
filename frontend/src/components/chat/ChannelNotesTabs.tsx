import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useBlocker } from "@tanstack/react-router";
import {
  ChevronDownIcon,
  ChevronUpIcon,
  CopyIcon,
  PlusIcon,
  RotateCcwIcon,
  SaveIcon,
  XIcon,
} from "lucide-react";

import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { Button } from "../ui/button";
import { Textarea } from "../ui/textarea";
import { MarkdownNote, toggleTaskMarker } from "./MarkdownNote";
import { useUsers, useChannelMembers } from "../../hooks/useRealtime";
import { toast } from "sonner";
import { detectComposerPlatform, shouldSendFromKey } from "./messages/Composer";
import {
  insertMention,
  mentionOptions,
  mentionQuery,
  type MentionOption,
} from "./messages/mentions";

interface Note {
  id: string;
  title: string;
  content: string;
  version: number;
  updated_at: string;
}

export function moveNoteToDropPosition<T>(
  items: T[],
  fromIndex: number,
  targetIndex: number,
  after: boolean,
): T[] {
  if (fromIndex < 0 || targetIndex < 0 || fromIndex >= items.length || targetIndex >= items.length)
    return items;
  const next = [...items];
  const [moved] = next.splice(fromIndex, 1);
  const adjustedTarget = targetIndex - (fromIndex < targetIndex ? 1 : 0);
  next.splice(adjustedTarget + (after ? 1 : 0), 0, moved!);
  return next;
}

export function ChannelNotesTabs({
  channelId,
  canWrite,
  archived,
  noteId,
  onNoteChange,
  onMessageTab,
}: {
  channelId: string;
  canWrite: boolean;
  archived: boolean;
  noteId?: string;
  onNoteChange: (id?: string) => void;
  onMessageTab: () => void;
}) {
  const { t } = useTranslation();
  const { config } = useCollections();
  const users = useUsers().data ?? [];
  const channelMembers = useChannelMembers(channelId).data ?? [];
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const path = `/api/channels/${channelId}/notes`;
  const [notes, setNotes] = useState<Note[]>([]);
  const [selected, setSelected] = useState<Note | null>(null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const previousNoteId = useRef(noteId);
  const navigation = useRef({ channelId, noteId, generation: 0 });
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [taskDraftContent, setTaskDraftContent] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [latest, setLatest] = useState<Note | null>(null);
  const [showDeleted, setShowDeleted] = useState(false);
  const [deleted, setDeleted] = useState<Note[]>([]);
  const dragging = useRef<string | null>(null);
  const dropPositionRef = useRef<{ id: string; after: boolean } | null>(null);
  const [dropPosition, setDropPosition] = useState<{ id: string; after: boolean } | null>(null);
  const requestGeneration = useRef(0);
  const loadController = useRef<AbortController | null>(null);
  const editingRef = useRef(editing);
  const canEditRef = useRef(canWrite && !archived);
  const reorderInFlight = useRef(false);
  const taskSaveInFlight = useRef(false);
  const taskSaveQueued = useRef(false);
  const taskDraftRef = useRef<string | null>(null);
  const composingRef = useRef(false);
  const [latestFetchFailed, setLatestFetchFailed] = useState(false);
  const [mentionRange, setMentionRange] = useState<{
    start: number;
    end: number;
    query: string;
  } | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const contentRef = useRef<HTMLTextAreaElement>(null);
  const memberIds = useMemo(
    () => new Set(channelMembers.map((row) => row.user_id)),
    [channelMembers],
  );
  const suggestions = useMemo(
    () => (mentionRange ? mentionOptions(users, memberIds, mentionRange.query) : []),
    [mentionRange, users, memberIds],
  );
  editingRef.current = editing;
  canEditRef.current = canWrite && !archived;
  if (navigation.current.channelId !== channelId || navigation.current.noteId !== noteId) {
    navigation.current = { channelId, noteId, generation: navigation.current.generation + 1 };
  }
  const dirty =
    Boolean(selected && editing && (title !== selected.title || content !== selected.content)) ||
    taskDraftContent !== null;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  function insertNoteMention(option: MentionOption) {
    if (!mentionRange) return;
    const inserted = insertMention(content, mentionRange, option.name);
    setContent(inserted.value);
    setMentionRange(null);
    requestAnimationFrame(() => {
      contentRef.current?.focus();
      contentRef.current?.setSelectionRange(inserted.caret, inserted.caret);
    });
  }
  useBlocker({
    shouldBlockFn: () => dirtyRef.current && !window.confirm(t("channels.notes.unsaved")),
    enableBeforeUnload: () => dirty,
  });

  const load = useCallback(async () => {
    loadController.current?.abort();
    const controller = new AbortController();
    loadController.current = controller;
    const generation = ++requestGeneration.current;
    try {
      const response = await client.requestWithTx<Note[]>(`${path}?summary=true`, {
        signal: controller.signal,
      });
      if (controller.signal.aborted || generation !== requestGeneration.current) return;
      if (taskSaveInFlight.current || taskDraftRef.current !== null) return;
      if (reorderInFlight.current) return;
      setNotes(response.data);
      if (noteId) {
        const current = response.data.find((n) => n.id === noteId);
        if (current && !editingRef.current) {
          const previous = selectedRef.current;
          const full =
            typeof current.content === "string"
              ? current
              : previous?.id === current.id && previous.version === current.version
                ? previous
                : (
                    await client.requestWithTx<Note>(`${path}/${current.id}`, {
                      signal: controller.signal,
                    })
                  ).data;
          if (
            controller.signal.aborted ||
            generation !== requestGeneration.current ||
            editingRef.current ||
            taskSaveInFlight.current ||
            taskDraftRef.current !== null
          )
            return;
          setSelected(full);
          setTitle(full.title);
          setContent(full.content);
          setError("");
        } else if (!current && !editingRef.current) {
          setSelected(null);
          setError(t("channels.notes.notFound"));
        }
      } else if (!editingRef.current) setSelected(null);
    } catch (cause) {
      if (!controller.signal.aborted && generation === requestGeneration.current)
        setError(cause instanceof Error ? cause.message : t("channels.notes.loadError"));
    } finally {
      if (loadController.current === controller) loadController.current = null;
    }
  }, [client, path, noteId, t]);
  const loadRef = useRef(load);
  loadRef.current = load;
  useEffect(() => {
    if (previousNoteId.current === noteId) return;
    previousNoteId.current = noteId;
    // Local tab selection and creation already installed the matching state.
    if (selected?.id === noteId) return;
    editingRef.current = false;
    dirtyRef.current = false;
    taskDraftRef.current = null;
    setTaskDraftContent(null);
    setSelected(null);
    setTitle("");
    setContent("");
    setEditing(false);
    setLatest(null);
    setLatestFetchFailed(false);
    setMentionRange(null);
    setError("");
  }, [noteId, selected?.id]);
  useEffect(() => {
    void load();
    return () => {
      requestGeneration.current += 1;
      loadController.current?.abort();
    };
  }, [load]);
  useEffect(() => {
    if (!noteId) return;
    const timer = window.setInterval(() => {
      if (!loadController.current) void load();
    }, 5000);
    return () => clearInterval(timer);
  }, [load, noteId]);
  useEffect(() => {
    if (canWrite && !archived) return;
    if (taskDraftRef.current !== null) {
      taskDraftRef.current = null;
      setTaskDraftContent(null);
      setContent(selected?.content ?? "");
      setEditing(false);
      setLatest(null);
      setLatestFetchFailed(false);
    }
  }, [canWrite, archived, selected]);
  useEffect(() => {
    if (!showDeleted) return;
    const controller = new AbortController();
    void client
      .requestWithTx<Note[]>(`${path}/deleted`, { signal: controller.signal })
      .then((r) => {
        if (!controller.signal.aborted) setDeleted(r.data);
      })
      .catch(() => {});
    return () => controller.abort();
  }, [client, path, showDeleted]);

  async function create() {
    if (busy || !canWrite || archived) return;
    if (dirty && !window.confirm(t("channels.notes.unsaved"))) return;
    const generation = navigation.current.generation;
    setBusy(true);
    requestGeneration.current += 1;
    setError("");
    try {
      const r = await client.post<Note>(path, { title: t("channels.notes.untitled"), content: "" });
      if (generation !== navigation.current.generation) return;
      requestGeneration.current += 1;
      loadController.current?.abort();
      taskDraftRef.current = null;
      setTaskDraftContent(null);
      setLatest(null);
      setLatestFetchFailed(false);
      setMentionRange(null);
      setNotes((old) => [...old, r.data]);
      setSelected(r.data);
      setTitle(r.data.title);
      setContent("");
      setEditing(true);
      dirtyRef.current = false;
      onNoteChange(r.data.id);
    } catch (e) {
      if (generation !== navigation.current.generation) return;
      setError(e instanceof Error ? e.message : t("common.writeError"));
    } finally {
      setBusy(false);
    }
  }
  async function save() {
    if (!selected || busy) return;
    if (!dirty) {
      setEditing(false);
      return;
    }
    const generation = navigation.current.generation;
    setBusy(true);
    requestGeneration.current += 1;
    setError("");
    try {
      const r = await client.patch<Note>(`${path}/${selected.id}`, {
        title,
        content,
        version: selected.version,
      });
      if (generation !== navigation.current.generation) return;
      setSelected(r.data);
      setNotes((old) => old.map((n) => (n.id === r.data.id ? r.data : n)));
      setTitle(r.data.title);
      setContent(r.data.content);
      setEditing(false);
      setLatest(null);
      setLatestFetchFailed(false);
      taskDraftRef.current = null;
      setTaskDraftContent(null);
      void load();
    } catch (e) {
      if (generation !== navigation.current.generation) return;
      if (e instanceof ApiError && e.status === 409) {
        setError(t("channels.notes.conflict"));
        try {
          const r = await client.requestWithTx<Note>(`${path}/${selected.id}`);
          if (generation !== navigation.current.generation) return;
          setLatest(r.data);
          setLatestFetchFailed(false);
        } catch (readError) {
          if (generation !== navigation.current.generation) return;
          setLatestFetchFailed(true);
          setError(readError instanceof Error ? readError.message : t("channels.notes.loadError"));
        }
      } else setError(e instanceof Error ? e.message : t("common.writeError"));
    } finally {
      setBusy(false);
    }
  }
  async function toggleTask(sourceOffset: number, checked: boolean) {
    if (!selected || (busy && !taskSaveInFlight.current) || !canWrite || archived) return;
    const baseContent = taskDraftRef.current ?? selected.content;
    const nextContent = toggleTaskMarker(baseContent, sourceOffset, checked);
    if (nextContent === baseContent) return;
    taskDraftRef.current = nextContent;
    setTaskDraftContent(nextContent);
    await persistTaskContent(nextContent);
  }
  async function persistTaskContent(nextContent: string) {
    if (!selected || (busy && !taskSaveInFlight.current) || !canWrite || archived) return;
    if (taskSaveInFlight.current) {
      taskSaveQueued.current = true;
      return;
    }
    const generation = navigation.current.generation;
    taskSaveInFlight.current = true;
    requestGeneration.current += 1;
    setBusy(true);
    setError("");
    let current = selected;
    let contentToSave = nextContent;
    let succeeded = false;
    try {
      do {
        taskSaveQueued.current = false;
        const response = await client.patch<Note>(`${path}/${current.id}`, {
          title: current.title,
          content: contentToSave,
          version: current.version,
        });
        if (generation !== navigation.current.generation || !canEditRef.current) return;
        current = response.data;
        setSelected(response.data);
        setNotes((old) => old.map((note) => (note.id === current.id ? response.data : note)));
        setTitle(response.data.title);
        setContent(response.data.content);
        succeeded = true;
        if (taskSaveQueued.current) contentToSave = taskDraftRef.current ?? response.data.content;
      } while (taskSaveQueued.current);
      if (succeeded && taskDraftRef.current === contentToSave) {
        taskDraftRef.current = null;
        setTaskDraftContent(null);
      }
    } catch (cause) {
      taskSaveQueued.current = false;
      if (generation !== navigation.current.generation) return;
      if (cause instanceof ApiError && cause.status === 409) {
        setError(t("channels.notes.conflict"));
        setLatestFetchFailed(false);
        try {
          const latestResponse = await client.requestWithTx<Note>(`${path}/${current.id}`);
          if (generation === navigation.current.generation) {
            setLatest(latestResponse.data);
            setLatestFetchFailed(false);
          }
        } catch (readError) {
          if (generation === navigation.current.generation) {
            setLatestFetchFailed(true);
            setError(
              readError instanceof Error
                ? `${t("channels.notes.conflict")} ${readError.message}`
                : t("channels.notes.conflict"),
            );
          }
        }
      } else if (generation === navigation.current.generation) {
        setError(cause instanceof Error ? cause.message : t("common.writeError"));
      }
    } finally {
      // A poll started while saving may still contain the previous version.
      requestGeneration.current += 1;
      loadController.current?.abort();
      taskSaveInFlight.current = false;
      setBusy(false);
      if (generation !== navigation.current.generation) void loadRef.current();
    }
  }
  async function reorder(from: string, to: string, after = false) {
    if (busy || reorderInFlight.current || !canWrite || archived || from === to) return;
    const a = notes.findIndex((n) => n.id === from);
    const b = notes.findIndex((n) => n.id === to);
    if (a < 0 || b < 0) return;
    const next = moveNoteToDropPosition(notes, a, b, after);
    if (next.every((note, index) => note.id === notes[index]?.id)) return;
    const generation = navigation.current.generation;
    reorderInFlight.current = true;
    requestGeneration.current += 1;
    setBusy(true);
    setNotes(next);
    let reorderError: string | null = null;
    try {
      await client.requestWithTx(`${path}/order`, {
        method: "PATCH",
        body: {
          expected_note_ids: notes.map((n) => n.id),
          note_ids: next.map((n) => n.id),
        },
      });
    } catch (e) {
      if (generation === navigation.current.generation) {
        reorderError = e instanceof Error ? e.message : t("common.writeError");
        setError(reorderError);
      }
    } finally {
      reorderInFlight.current = false;
      setBusy(false);
      void loadRef.current().then(() => {
        if (reorderError && generation === navigation.current.generation) setError(reorderError);
      });
    }
  }
  async function restore(note: Note) {
    if (busy) return;
    const generation = navigation.current.generation;
    setBusy(true);
    try {
      await client.post<Note>(`${path}/${note.id}/restore`, {});
      if (generation !== navigation.current.generation) return;
      setDeleted((d) => d.filter((n) => n.id !== note.id));
      void load();
    } catch (e) {
      if (generation !== navigation.current.generation) return;
      setError(e instanceof Error ? e.message : t("common.writeError"));
    } finally {
      setBusy(false);
    }
  }
  async function remove() {
    if (!selected || busy || !window.confirm(t("channels.notes.deleteConfirm"))) return;
    const generation = navigation.current.generation;
    setBusy(true);
    setError("");
    try {
      await client.delete(`${path}/${selected.id}`, { body: { version: selected.version } });
      if (generation !== navigation.current.generation) return;
      requestGeneration.current += 1;
      loadController.current?.abort();
      setNotes((all) => all.filter((note) => note.id !== selected.id));
      setSelected(null);
      setEditing(false);
      setTitle("");
      setContent("");
      setLatest(null);
      setLatestFetchFailed(false);
      taskDraftRef.current = null;
      setTaskDraftContent(null);
      onNoteChange(undefined);
    } catch (e) {
      if (generation !== navigation.current.generation) return;
      setError(e instanceof Error ? e.message : t("common.writeError"));
    } finally {
      setBusy(false);
    }
  }
  function select(note: Note) {
    if (busy || taskSaveInFlight.current) return;
    if (dirty && !window.confirm(t("channels.notes.unsaved"))) return;
    const full =
      typeof note.content === "string"
        ? note
        : selectedRef.current?.id === note.id
          ? selectedRef.current
          : null;
    setSelected(full);
    setTitle(note.title);
    setContent(full?.content ?? "");
    setEditing(false);
    setLatest(null);
    setLatestFetchFailed(false);
    setMentionRange(null);
    setError("");
    taskDraftRef.current = null;
    setTaskDraftContent(null);
    dirtyRef.current = false;
    onNoteChange(note.id);
  }
  function selectMessage() {
    if (busy || taskSaveInFlight.current) return;
    if (dirty && !window.confirm(t("channels.notes.unsaved"))) return;
    dirtyRef.current = false;
    onNoteChange(undefined);
    setSelected(null);
    taskDraftRef.current = null;
    setTaskDraftContent(null);
    setEditing(false);
    setLatest(null);
    setLatestFetchFailed(false);
    setMentionRange(null);
    setError("");
    onMessageTab();
  }

  return (
    <section
      className={`flex min-h-0 flex-col ${noteId ? "flex-1" : "shrink-0"}`}
      aria-label={t("channels.notes.title")}
    >
      <div className="flex shrink-0 items-center gap-1 overflow-x-auto border-b px-2 py-1">
        <div
          role="tablist"
          aria-label={t("channels.notes.title")}
          aria-owns={[
            `message-tab-${channelId}`,
            ...notes.map((note) => `note-tab-${note.id}`),
          ].join(" ")}
        />
        <button
          id={`message-tab-${channelId}`}
          role="tab"
          aria-selected={!noteId}
          disabled={busy}
          className="shrink-0 rounded px-3 py-2 text-sm hover:bg-muted"
          onClick={selectMessage}
        >
          {t("channels.notes.messages")}
        </button>
        {notes.map((note, index) => (
          <div
            key={note.id}
            draggable={canWrite && !archived}
            onDragStart={() => {
              dragging.current = note.id;
            }}
            onDragOver={(e) => {
              e.preventDefault();
              const bounds = e.currentTarget.getBoundingClientRect();
              const after = e.clientX >= bounds.left + bounds.width / 2;
              const nextPosition = { id: note.id, after };
              dropPositionRef.current = nextPosition;
              setDropPosition(nextPosition);
            }}
            onDrop={() => {
              const position = dropPositionRef.current;
              if (dragging.current && position?.id === note.id)
                void reorder(dragging.current, note.id, position.after);
              dragging.current = null;
              dropPositionRef.current = null;
              setDropPosition(null);
            }}
            onDragEnd={() => {
              dragging.current = null;
              dropPositionRef.current = null;
              setDropPosition(null);
            }}
            className={`flex shrink-0 items-center ${dropPosition?.id === note.id ? (dropPosition.after ? "border-r-2 border-primary" : "border-l-2 border-primary") : ""}`}
          >
            <button
              id={`note-tab-${note.id}`}
              role="tab"
              aria-selected={noteId === note.id}
              disabled={busy}
              className="max-w-48 truncate rounded px-3 py-2 text-sm hover:bg-muted"
              onClick={() => select(note)}
            >
              {selected?.id === note.id && editing ? title || note.title : note.title}
            </button>
            {canWrite && !archived ? (
              <span className="flex md:sr-only md:focus-within:not-sr-only">
                <button
                  aria-label={`${t("channels.notes.moveUp")} ${note.title}`}
                  disabled={busy || index === 0}
                  onClick={() => void reorder(note.id, notes[index - 1]!.id, false)}
                  className="p-2 disabled:opacity-30"
                >
                  <ChevronUpIcon className="size-3" />
                </button>
                <button
                  aria-label={`${t("channels.notes.moveDown")} ${note.title}`}
                  disabled={busy || index === notes.length - 1}
                  onClick={() => void reorder(note.id, notes[index + 1]!.id, true)}
                  className="p-2 disabled:opacity-30"
                >
                  <ChevronDownIcon className="size-3" />
                </button>
              </span>
            ) : null}
          </div>
        ))}
        {canWrite && !archived ? (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("channels.notes.new")}
            disabled={busy}
            onClick={() => void create()}
          >
            <PlusIcon />
          </Button>
        ) : null}
        {canWrite && !archived ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => setShowDeleted((v) => !v)}
          >
            {t("channels.notes.deleted")}
          </Button>
        ) : null}
      </div>
      {error ? (
        <p role="alert" className="border-b px-4 py-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
      {latestFetchFailed ? (
        <Button
          variant="outline"
          className="m-3 self-start"
          disabled={busy}
          onClick={async () => {
            if (!selected) return;
            const generation = navigation.current.generation;
            setBusy(true);
            try {
              const response = await client.requestWithTx<Note>(`${path}/${selected.id}`);
              if (generation !== navigation.current.generation) return;
              setLatest(response.data);
              setLatestFetchFailed(false);
              setError(t("channels.notes.conflict"));
            } catch (cause) {
              if (generation !== navigation.current.generation) return;
              setError(cause instanceof Error ? cause.message : t("channels.notes.loadError"));
            } finally {
              setBusy(false);
            }
          }}
        >
          {t("channels.notes.retry")}
        </Button>
      ) : null}
      {showDeleted ? (
        <div className="flex flex-wrap gap-2 border-b p-3">
          {deleted.map((note) => (
            <Button
              key={note.id}
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => void restore(note)}
            >
              <RotateCcwIcon />
              {t("channels.notes.restore")}: {note.title}
            </Button>
          ))}
        </div>
      ) : null}
      {noteId && !selected ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6">
          <p className="text-sm text-muted-foreground">{error || t("channels.notes.notFound")}</p>
          <Button variant="outline" onClick={selectMessage}>
            {t("channels.notes.messages")}
          </Button>
        </div>
      ) : null}
      {noteId && selected ? (
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {editing ? (
            <div className="mx-auto flex max-w-4xl flex-col gap-3">
              <input
                aria-label={t("channels.notes.noteTitle")}
                maxLength={200}
                value={title}
                disabled={busy}
                onChange={(e) => setTitle(e.target.value)}
                onCompositionStart={() => {
                  composingRef.current = true;
                }}
                onCompositionEnd={() => {
                  composingRef.current = false;
                }}
                onKeyDown={(event) => {
                  if (
                    shouldSendFromKey(
                      {
                        key: event.key,
                        metaKey: event.metaKey,
                        ctrlKey: event.ctrlKey,
                        shiftKey: event.shiftKey,
                        isComposing:
                          composingRef.current ||
                          event.nativeEvent.isComposing ||
                          event.nativeEvent.keyCode === 229,
                      },
                      detectComposerPlatform(),
                    )
                  ) {
                    event.preventDefault();
                    void save();
                  }
                }}
                className="rounded border bg-background px-3 py-2 text-xl font-semibold"
              />
              <Textarea
                ref={contentRef}
                aria-label={t("channels.notes.content")}
                className="min-h-64 resize-y"
                maxLength={100000}
                value={content}
                disabled={busy}
                onChange={(e) => {
                  const value = e.target.value;
                  setContent(value);
                  setMentionRange(mentionQuery(value, e.target.selectionStart));
                  setMentionIndex(0);
                }}
                onCompositionStart={() => {
                  composingRef.current = true;
                }}
                onCompositionEnd={() => {
                  composingRef.current = false;
                }}
                onKeyDown={(event) => {
                  // IME owns Enter, arrows, Tab and Escape while composing. Safari may only report 229.
                  if (
                    composingRef.current ||
                    event.nativeEvent.isComposing ||
                    event.nativeEvent.keyCode === 229
                  )
                    return;
                  if (
                    mentionRange &&
                    suggestions.length &&
                    !event.metaKey &&
                    !event.ctrlKey &&
                    (event.key === "ArrowDown" || event.key === "ArrowUp")
                  ) {
                    event.preventDefault();
                    setMentionIndex(
                      (index) =>
                        (index + (event.key === "ArrowDown" ? 1 : suggestions.length - 1)) %
                        suggestions.length,
                    );
                    return;
                  } else if (
                    mentionRange &&
                    suggestions.length &&
                    !event.metaKey &&
                    !event.ctrlKey &&
                    (event.key === "Enter" || event.key === "Tab")
                  ) {
                    event.preventDefault();
                    insertNoteMention(suggestions[mentionIndex]!);
                    return;
                  } else if (
                    mentionRange &&
                    suggestions.length &&
                    !event.metaKey &&
                    !event.ctrlKey &&
                    event.key === "Escape"
                  ) {
                    setMentionRange(null);
                    return;
                  }
                  if (
                    shouldSendFromKey(
                      {
                        key: event.key,
                        metaKey: event.metaKey,
                        ctrlKey: event.ctrlKey,
                        shiftKey: event.shiftKey,
                        isComposing: event.nativeEvent.isComposing,
                      },
                      detectComposerPlatform(),
                    )
                  ) {
                    event.preventDefault();
                    void save();
                  }
                }}
              />
              {mentionRange && suggestions.length ? (
                <ul
                  role="listbox"
                  aria-label={t("mentions.suggestions")}
                  className="max-h-40 overflow-y-auto rounded-md border bg-popover p-1"
                >
                  {suggestions.map((option, index) => (
                    <li key={option.name}>
                      <button
                        type="button"
                        role="option"
                        aria-selected={index === mentionIndex}
                        className="w-full rounded px-2 py-1 text-left text-sm hover:bg-muted"
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => insertNoteMention(option)}
                      >
                        @{option.name}
                        {option.label !== option.name ? ` · ${option.label}` : ""}
                      </button>
                    </li>
                  ))}
                </ul>
              ) : null}
              {latest ? (
                <div role="status" className="rounded border p-3">
                  <b>
                    {t("channels.notes.latestVersion")}: {latest.title}
                  </b>
                  <MarkdownNote content={latest.content} />
                  <Button
                    variant="outline"
                    onClick={() => {
                      setSelected(latest);
                      setTitle(latest.title);
                      setContent(latest.content);
                      setLatest(null);
                      taskDraftRef.current = null;
                      setTaskDraftContent(null);
                    }}
                  >
                    {" "}
                    {t("channels.notes.useLatest")}
                  </Button>
                  <Button
                    variant="destructive"
                    onClick={() => {
                      setSelected(latest);
                      setLatest(null);
                    }}
                  >
                    {t("channels.notes.overwriteLatest")}
                  </Button>
                </div>
              ) : null}
              {busy || error || dirty ? (
                <p role="status" aria-live="polite" className="text-sm text-muted-foreground">
                  {busy
                    ? t("channels.notes.saving")
                    : error
                      ? t("channels.notes.saveFailed")
                      : t("channels.notes.unsavedChanges")}
                </p>
              ) : null}
              <div className="flex gap-2">
                <Button disabled={busy || !title.trim()} onClick={() => void save()}>
                  <SaveIcon />
                  {t("channels.notes.save")}
                </Button>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => {
                    if (!dirty || window.confirm(t("channels.notes.unsaved"))) {
                      setTitle(selected.title);
                      setContent(selected.content);
                      setEditing(false);
                      setLatest(null);
                      setLatestFetchFailed(false);
                      taskDraftRef.current = null;
                      setTaskDraftContent(null);
                    }
                  }}
                >
                  <XIcon />
                  {t("common.cancelShort")}
                </Button>
              </div>
            </div>
          ) : (
            <div className="mx-auto max-w-4xl">
              <div className="mb-4 flex items-center justify-between gap-2">
                <h2 className="text-xl font-semibold">{selected.title}</h2>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  aria-label={t("channels.notes.copyUrl")}
                  onClick={async () => {
                    const url = new URL(window.location.href);
                    url.searchParams.set("note", selected.id);
                    try {
                      await navigator.clipboard.writeText(url.href);
                      toast.success(t("channels.notes.copied"));
                    } catch {
                      toast.error(t("channels.notes.copyError"));
                    }
                  }}
                >
                  <CopyIcon aria-hidden="true" />
                  {t("channels.notes.copyUrl")}
                </Button>
              </div>
              <article className="prose prose-sm max-w-none">
                <div className="break-words">
                  <MarkdownNote
                    content={taskDraftContent ?? selected.content}
                    taskCheckboxes={canWrite && !archived && (!busy || taskSaveInFlight.current)}
                    onTaskToggle={(sourceOffset, checked) => void toggleTask(sourceOffset, checked)}
                  />
                  {taskDraftContent !== null && !busy ? (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => void persistTaskContent(taskDraftContent)}
                    >
                      {t("channels.notes.retry")}
                    </Button>
                  ) : null}
                  {taskSaveInFlight.current ? (
                    <p role="status" className="text-sm text-muted-foreground">
                      {t("channels.notes.saving")}
                    </p>
                  ) : null}
                  {!taskDraftContent && error ? (
                    <p role="status" aria-live="polite" className="text-sm text-muted-foreground">
                      {t("channels.notes.saveFailed")}
                    </p>
                  ) : null}
                </div>
              </article>
              {canWrite && !archived ? (
                <div className="mt-4 flex gap-2">
                  <Button
                    variant="outline"
                    onClick={() => {
                      setContent(taskDraftContent ?? selected.content);
                      setEditing(true);
                    }}
                  >
                    {t("channels.notes.edit")}
                  </Button>
                  <Button variant="destructive" disabled={busy} onClick={() => void remove()}>
                    {t("channels.notes.delete")}
                  </Button>
                </div>
              ) : null}
            </div>
          )}
        </div>
      ) : null}
    </section>
  );
}
