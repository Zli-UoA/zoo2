import { Textarea } from "../../ui/textarea";
import { ComposerField, composerTextareaClassName } from "./ComposerField";
import { useAutosizeTextarea } from "../../../hooks/useAutosizeTextarea";
import { useTranslation } from "react-i18next";
import { useEffect, useId, useLayoutEffect, useRef, useState, type ClipboardEvent } from "react";
import { PaperclipIcon, SendIcon, XIcon } from "lucide-react";

import type { TFunction } from "i18next";
import {
  MAX_ATTACHMENTS_PER_MESSAGE,
  MAX_ATTACHMENT_FILE_SIZE,
  attachmentFileName,
  attachmentFilesFromClipboard,
  attachmentRejection,
  formatAttachmentSize,
  isInlineImage,
  resolveAttachmentContentType,
} from "../../../lib/attachments";
import type { UploadedAttachment } from "../../../lib/api/attachments";
import { useLocale } from "../../i18n/LocaleProvider";
import { useMediaQuery } from "../../layout/useMediaQuery";
import { Button } from "../../ui/button";
import { Spinner } from "../../ui/spinner";

import { insertMention, mentionOptions, mentionQuery, type MentionOption } from "./mentions";
import { insertMfm, mfmOptions, mfmQuery } from "./mfmSuggestions";
import type { MFM_FUNCTIONS } from "./mfmFunctions";
import type { UserRow } from "../../../lib/electric/types";

import { useMfmChannels } from "./MfmChannels";
import { composerQuery, insertSuggestion, type CommandSuggestion } from "./composerSuggestions";

const MAX_COMPOSER_HEIGHT = 200;

export type ComposerPlatform = "mac" | "windows" | "linux" | "unknown";

export interface ComposerKeyInput {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  isComposing: boolean;
}

/** プラットフォーム文字列（navigator.platform / userAgent）から送信キーの系統を判定する。 */
export function resolveComposerPlatform(platformHint: string): ComposerPlatform {
  const hint = platformHint.toLowerCase();
  if (hint.includes("mac")) {
    return "mac";
  }
  if (hint.includes("win")) {
    return "windows";
  }
  if (hint.includes("linux") || hint.includes("x11")) {
    return "linux";
  }
  return "unknown";
}

export function detectComposerPlatform(): ComposerPlatform {
  if (typeof navigator === "undefined") {
    return "unknown";
  }
  return resolveComposerPlatform(`${navigator.platform} ${navigator.userAgent}`);
}

/**
 * 送信キーの判定。macOS は Cmd+Enter、Windows / Linux は Ctrl+Enter。
 * Enter / Shift+Enter は改行に任せ、IME 変換中（isComposing）は送信しない。
 */
export function shouldSendFromKey(input: ComposerKeyInput, platform: ComposerPlatform): boolean {
  if (input.key !== "Enter" || input.isComposing || input.shiftKey) {
    return false;
  }
  if (platform === "mac") {
    return input.metaKey;
  }
  if (platform === "windows" || platform === "linux") {
    return input.ctrlKey;
  }
  return input.metaKey || input.ctrlKey;
}

/** 空白のみの下書きでも添付があれば送信できる。アップロード中は送信しない。 */
export function canSubmitComposer(
  value: string,
  disabled: boolean,
  attachmentCount = 0,
  isUploading = false,
): boolean {
  return !disabled && !isUploading && (value.trim().length > 0 || attachmentCount > 0);
}

/** キーボード操作を案内できないタッチ環境では案内を表示しない。 */
export function composerSendHint(
  platform: ComposerPlatform,
  t: TFunction,
  isTouch = false,
): string {
  if (isTouch) {
    return "";
  }
  if (platform === "mac") {
    return t("composer.mac");
  }
  if (platform === "windows" || platform === "linux") {
    return t("composer.ctrl");
  }
  return t("composer.other");
}

interface ComposerAttachmentDraft {
  key: string;
  file: File;
  file_name: string;
  file_size: number;
  content_type: string;
  preview_url: string | null;
  status: "uploading" | "ready";
  attachment_id?: string;
}

interface ComposerProps {
  commands?: readonly CommandSuggestion[];
  onRequestCommands?: () => void;
  mentionUsers?: UserRow[];
  mentionMemberIds?: ReadonlySet<string>;
  /** 送信を実行し、成功したかを返す。false なら下書きを復元する。 */
  onTyping?: (active: boolean) => void;
  onSend: (content: string, commandAppId?: string, attachmentIds?: string[]) => Promise<boolean>;
  /** ファイルをアップロードして添付 ID を返す。未指定なら添付 UI を出さない。 */
  onUploadAttachment?: (file: File) => Promise<UploadedAttachment>;
  disabled?: boolean;
  placeholder?: string;
  ariaLabel?: string;
}

/**
 * メッセージ入力。複数行 autosize、⌘/Ctrl+Enter 送信 / Enter 改行。
 * 送信は楽観更新に任せ、入力欄は即座にクリアする。確定に失敗したら下書きを戻す。
 */
export function Composer({
  onTyping,
  commands = [],
  onRequestCommands,
  mentionUsers = [],
  mentionMemberIds = new Set(),
  onSend,
  onUploadAttachment,
  disabled = false,
  placeholder,
  ariaLabel,
}: ComposerProps) {
  const { t } = useTranslation();
  const { locale } = useLocale();
  const resolvedPlaceholder = placeholder ?? t("messages.compose");
  const resolvedAriaLabel = ariaLabel ?? t("messages.compose");
  const [value, setValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [activeOption, setActiveOption] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [drafts, setDrafts] = useState<ComposerAttachmentDraft[]>([]);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const draftsRef = useRef<ComposerAttachmentDraft[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const composingRef = useRef(false);
  const draftVersion = useRef(0);
  const pendingSuggestionCaret = useRef<number | null>(null);
  const listId = useId();
  const channels = useMfmChannels();
  const [selectedCommand, setSelectedCommand] = useState<CommandSuggestion | null>(null);
  const suggest = !disabled && !dismissed && caret >= 0;
  const mfmRange = suggest ? mfmQuery(value, caret) : null;
  const commandRange = suggest ? composerQuery(value, caret, "command") : null;
  const channelRange = suggest ? composerQuery(value, caret, "channel") : null;
  const range = suggest && !mfmRange ? mentionQuery(value, caret) : null;
  const requestingCommands = !!commandRange;
  useEffect(() => {
    if (requestingCommands) onRequestCommands?.();
  }, [requestingCommands, onRequestCommands]);
  const kind = commandRange
    ? "commands"
    : channelRange
      ? "channelSuggestions"
      : mfmRange
        ? "mfm"
        : "mentions";
  const options: (MentionOption & { app_id?: string })[] = commandRange
    ? commands
        .filter((command) => command.name.toLowerCase().startsWith(commandRange.query))
        .map((command) => ({ name: command.name, label: command.app_name, app_id: command.app_id }))
    : channelRange
      ? [...channels.keys()]
          .filter((name) => name.normalize("NFKC").toLowerCase().includes(channelRange.query))
          .sort((a, b) => a.localeCompare(b))
          .slice(0, 20)
          .map((name) => ({ name, label: "" }))
      : mfmRange
        ? mfmOptions(mfmRange.query).map((name) => ({
            name,
            label: t(`mfm.functions.${name}`),
          }))
        : range
          ? mentionOptions(mentionUsers, mentionMemberIds, range.query)
          : [];
  const selectedOption = Math.min(activeOption, Math.max(0, options.length - 1));
  function selectOption(option: (typeof options)[number]) {
    if (composingRef.current) return;
    const next = commandRange
      ? insertSuggestion(value, commandRange, `/${option.name}`)
      : channelRange
        ? insertSuggestion(value, channelRange, `#${option.name}`)
        : mfmRange
          ? insertMfm(value, mfmRange, option.name as (typeof MFM_FUNCTIONS)[number])
          : range
            ? insertMention(value, range, option.name)
            : null;
    if (!next) return;
    if (commandRange)
      setSelectedCommand(
        commands.find(
          (command) => command.name === option.name && command.app_id === option.app_id,
        ) ?? null,
      );
    draftVersion.current++;
    setValue(next.value);
    setCaret(next.caret);
    setDismissed(true);
    onTyping?.(true);
    pendingSuggestionCaret.current = next.caret;
  }
  const [platform] = useState(() => detectComposerPlatform());
  // タッチ端末には Ctrl / Cmd が無いため、キーボードショートカットを案内しない。
  const isTouch = useMediaQuery("(hover: none)");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  // Apply the suggestion caret during commit, before another input event can
  // arrive. A deferred animation frame can move it into newly typed text.
  useLayoutEffect(() => {
    const position = pendingSuggestionCaret.current;
    if (position === null) return;
    pendingSuggestionCaret.current = null;
    textareaRef.current?.focus();
    textareaRef.current?.setSelectionRange(position, position);
  });
  useLayoutEffect(() => {
    listRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  }, [selectedOption, options.length]);

  useAutosizeTextarea(textareaRef, value, MAX_COMPOSER_HEIGHT);

  function updateDraft(element: HTMLTextAreaElement, isComposing = composingRef.current) {
    setCaret(isComposing ? element.selectionEnd : element.selectionStart);
    setActiveOption(0);
    setDismissed(false);
    draftVersion.current++;
    if (
      selectedCommand &&
      element.value !== `/${selectedCommand.name}` &&
      !element.value.startsWith(`/${selectedCommand.name} `)
    )
      setSelectedCommand(null);
    setValue(element.value);
    onTyping?.(element.value.trim().length > 0);
  }

  function updateDrafts(next: ComposerAttachmentDraft[]) {
    draftsRef.current = next;
    setDrafts(next);
  }

  function revokeDrafts(entries: ComposerAttachmentDraft[]) {
    for (const draft of entries) {
      if (draft.preview_url) {
        URL.revokeObjectURL(draft.preview_url);
      }
    }
  }

  function removeDraft(key: string) {
    const draft = draftsRef.current.find((candidate) => candidate.key === key);
    if (!draft) {
      return;
    }
    updateDrafts(draftsRef.current.filter((candidate) => candidate.key !== key));
    if (draft.preview_url) {
      URL.revokeObjectURL(draft.preview_url);
    }
  }

  function rejectionMessage(reason: ReturnType<typeof attachmentRejection>): string | null {
    if (reason === "too-many") {
      return t("attachments.tooMany", { max: MAX_ATTACHMENTS_PER_MESSAGE });
    }
    if (reason === "too-large") {
      return t("attachments.tooLarge", {
        max: Math.floor(MAX_ATTACHMENT_FILE_SIZE / (1024 * 1024)),
      });
    }
    if (reason === "empty") {
      return t("attachments.empty");
    }
    if (reason === "unsupported") {
      return t("attachments.unsupported");
    }
    return null;
  }

  function addFiles(files: File[]) {
    const upload = onUploadAttachment;
    if (!upload || disabled || files.length === 0) {
      return;
    }
    let error: string | null = null;
    const accepted: { draft: ComposerAttachmentDraft; upload: typeof upload }[] = [];
    for (const file of files) {
      const reason = attachmentRejection(file, draftsRef.current.length + accepted.length);
      if (reason) {
        error ??= rejectionMessage(reason);
        continue;
      }
      const contentType = resolveAttachmentContentType(file);
      if (!contentType) {
        error ??= t("attachments.unsupported");
        continue;
      }
      accepted.push({
        upload,
        draft: {
          key: crypto.randomUUID(),
          file,
          file_name: attachmentFileName(file, contentType),
          file_size: file.size,
          content_type: contentType,
          preview_url: isInlineImage(contentType) ? URL.createObjectURL(file) : null,
          status: "uploading",
        },
      });
    }
    setAttachmentError(error);
    if (accepted.length === 0) {
      return;
    }
    updateDrafts([...draftsRef.current, ...accepted.map((entry) => entry.draft)]);
    for (const entry of accepted) {
      void uploadDraft(entry.draft, entry.upload);
    }
  }

  async function uploadDraft(
    draft: ComposerAttachmentDraft,
    upload: (file: File) => Promise<UploadedAttachment>,
  ) {
    try {
      const uploaded = await upload(draft.file);
      updateDrafts(
        draftsRef.current.map((candidate) =>
          candidate.key === draft.key
            ? {
                ...candidate,
                status: "ready" as const,
                attachment_id: uploaded.id,
                file_name: uploaded.file_name,
                file_size: uploaded.file_size,
                content_type: uploaded.content_type,
              }
            : candidate,
        ),
      );
    } catch {
      removeDraft(draft.key);
      setAttachmentError(t("attachments.uploadError"));
    }
  }

  function handlePaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    if (!onUploadAttachment || disabled) {
      return;
    }
    const files = attachmentFilesFromClipboard(event.clipboardData);
    if (files.length === 0) {
      return;
    }
    event.preventDefault();
    addFiles(files);
  }

  async function submit() {
    const content = value.trim();
    const ready = draftsRef.current.filter(
      (draft) => draft.status === "ready" && draft.attachment_id,
    );
    const uploading = draftsRef.current.some((draft) => draft.status === "uploading");
    if (composingRef.current || !canSubmitComposer(value, disabled, ready.length, uploading)) {
      return;
    }
    const version = draftVersion.current;
    const restoreDrafts = draftsRef.current;
    const attachmentIds = ready.map((draft) => draft.attachment_id!);
    const commandAppId =
      selectedCommand &&
      (content === `/${selectedCommand.name}` || content.startsWith(`/${selectedCommand.name} `))
        ? selectedCommand.app_id
        : undefined;
    setAttachmentError(null);
    setSelectedCommand(null);
    setValue("");
    onTyping?.(false);
    updateDrafts([]);
    let sent = false;
    try {
      sent =
        attachmentIds.length > 0
          ? await onSend(content, undefined, attachmentIds)
          : commandAppId
            ? await onSend(content, commandAppId)
            : await onSend(content);
    } catch {
      sent = false;
    }
    if (!sent) {
      if (draftVersion.current === version) setSelectedCommand(selectedCommand);
      // 失敗時は下書きを戻す。ただし待っている間に次の入力を始めていたら邪魔しない。
      setValue((current) => (current.length > 0 ? current : content));
      updateDrafts([...draftsRef.current, ...restoreDrafts]);
      return;
    }
    revokeDrafts(restoreDrafts);
  }

  const readyAttachmentCount = drafts.filter(
    (draft) => draft.status === "ready" && draft.attachment_id,
  ).length;
  const isUploading = drafts.some((draft) => draft.status === "uploading");

  useLayoutEffect(() => {
    return () => {
      revokeDrafts(draftsRef.current);
    };
  }, []);

  return (
    <form
      autoComplete="off"
      className="shrink-0 border-t px-3 py-3"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      {drafts.length > 0 ? (
        <ul className="mb-2 flex flex-wrap gap-2" aria-label={t("attachments.selected")}>
          {drafts.map((draft) => {
            const isImage = isInlineImage(draft.content_type) && draft.preview_url;
            return (
              <li
                key={draft.key}
                className={
                  isImage
                    ? "relative size-20 overflow-hidden rounded-lg border bg-muted"
                    : "relative flex h-20 w-56 items-center gap-2 overflow-hidden rounded-lg border bg-muted/30 p-2 pr-9"
                }
              >
                {isImage ? (
                  <img
                    src={draft.preview_url!}
                    alt={draft.file_name}
                    className="size-full object-cover"
                  />
                ) : (
                  <>
                    <PaperclipIcon
                      className="size-5 shrink-0 text-muted-foreground"
                      aria-hidden="true"
                    />
                    <div className="min-w-0">
                      <p className="truncate text-xs font-medium">{draft.file_name}</p>
                      <p className="text-[11px] text-muted-foreground">
                        {formatAttachmentSize(draft.file_size, locale)}
                      </p>
                    </div>
                  </>
                )}
                {draft.status === "uploading" ? (
                  <Spinner
                    label={t("attachments.uploading")}
                    className="absolute inset-0 bg-background/60"
                  />
                ) : null}
                <button
                  type="button"
                  className="absolute right-1 top-1 grid size-6 place-items-center rounded-full bg-background/90 text-foreground shadow-sm hover:bg-background"
                  aria-label={t("attachments.remove", { name: draft.file_name })}
                  title={t("attachments.remove", { name: draft.file_name })}
                  onClick={() => removeDraft(draft.key)}
                >
                  <XIcon className="size-3.5" />
                </button>
              </li>
            );
          })}
        </ul>
      ) : null}
      {attachmentError ? (
        <p role="alert" className="mb-2 px-1 text-xs text-destructive">
          {attachmentError}
        </p>
      ) : null}
      {options.length > 0 ? (
        <ul
          id={listId}
          ref={listRef}
          role="listbox"
          aria-label={t(
            kind === "commands"
              ? "composer.commands"
              : kind === "channelSuggestions"
                ? "composer.channels"
                : `${kind}.suggestions`,
          )}
          className="mb-2 max-h-48 overflow-y-auto rounded-lg border bg-popover p-1"
        >
          {options.map((option, index) => (
            <li
              key={`${option.app_id ?? ""}:${option.name}`}
              id={`${listId}-${index}`}
              role="option"
              aria-selected={index === selectedOption}
              tabIndex={-1}
              className={`flex cursor-pointer gap-2 rounded px-2 py-1 text-left text-sm outline-none ${index === selectedOption ? "bg-accent" : ""}`}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => selectOption(option)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  selectOption(option);
                }
              }}
            >
              <span className="shrink-0 font-mono">
                {commandRange
                  ? `/${option.name}`
                  : channelRange
                    ? `#${option.name}`
                    : mfmRange
                      ? `$[${option.name}]`
                      : `@${option.name}`}
              </span>
              <span className="truncate text-muted-foreground">{option.label}</span>
            </li>
          ))}
        </ul>
      ) : null}
      <ComposerField>
        {onUploadAttachment ? (
          <>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              hidden
              onChange={(event) => {
                const files = Array.from(event.currentTarget.files ?? []);
                event.currentTarget.value = "";
                addFiles(files);
              }}
            />
            <Button
              type="button"
              variant="ghost"
              size="icon-lg"
              className="mb-1 size-11! shrink-0 md:size-9!"
              aria-label={t("attachments.add")}
              title={t("attachments.add")}
              disabled={disabled || drafts.length >= MAX_ATTACHMENTS_PER_MESSAGE}
              onClick={() => fileInputRef.current?.click()}
            >
              <PaperclipIcon />
            </Button>
          </>
        ) : null}
        <Textarea
          ref={textareaRef}
          rows={1}
          value={value}
          disabled={disabled}
          placeholder={resolvedPlaceholder}
          aria-label={resolvedAriaLabel}
          className={`${composerTextareaClassName} min-h-11 px-2 py-2.5`}
          aria-autocomplete="list"
          aria-controls={options.length ? listId : undefined}
          aria-activedescendant={options.length ? `${listId}-${selectedOption}` : undefined}
          onPaste={handlePaste}
          onCompositionStart={() => {
            composingRef.current = true;
            setDismissed(false);
          }}
          onCompositionEnd={(event) => {
            composingRef.current = false;
            updateDraft(event.currentTarget, false);
          }}
          onInput={(event) => {
            if (composingRef.current || (event.nativeEvent as InputEvent).isComposing)
              updateDraft(event.currentTarget, true);
          }}
          onSelect={(event) =>
            setCaret(
              composingRef.current ||
                event.currentTarget.selectionStart === event.currentTarget.selectionEnd
                ? event.currentTarget.selectionEnd
                : -1,
            )
          }
          onChange={(event) => updateDraft(event.currentTarget)}
          onBlur={() => {
            setDismissed(true);
            onTyping?.(false);
          }}
          onKeyDown={(event) => {
            // IME owns Enter, arrows, Tab and Escape while composing. Safari may only report 229.
            if (
              composingRef.current ||
              event.nativeEvent.isComposing ||
              event.nativeEvent.keyCode === 229
            )
              return;
            if (options.length && !event.metaKey && !event.ctrlKey) {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                setActiveOption(
                  (selectedOption + (event.key === "ArrowDown" ? 1 : -1) + options.length) %
                    options.length,
                );
                return;
              }
              if ((event.key === "Enter" || event.key === "Tab") && !event.shiftKey) {
                event.preventDefault();
                selectOption(options[selectedOption]!);
                return;
              }
              if (event.key === "Escape") {
                event.preventDefault();
                setDismissed(true);
                return;
              }
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
                platform,
              )
            ) {
              event.preventDefault();
              void submit();
            }
          }}
        />
        <Button
          type="submit"
          size="icon-lg"
          className="mb-1 size-11! md:size-9!"
          disabled={!canSubmitComposer(value, disabled, readyAttachmentCount, isUploading)}
          aria-label={t("messages.send")}
          title={t("messages.send")}
        >
          <SendIcon />
        </Button>
      </ComposerField>
      {!isTouch ? (
        <p className="mt-1.5 px-1 text-[11px] text-muted-foreground">
          {composerSendHint(platform, t, isTouch)}
        </p>
      ) : null}
    </form>
  );
}
