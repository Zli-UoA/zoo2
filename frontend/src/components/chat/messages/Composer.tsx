import { useTranslation } from "react-i18next";
import { useId, useLayoutEffect, useRef, useState } from "react";
import { SendIcon } from "lucide-react";

import type { TFunction } from "i18next";
import { useMediaQuery } from "../../layout/useMediaQuery";
import { Button } from "../../ui/button";
import { Textarea } from "../../ui/textarea";

import { insertMention, mentionOptions, mentionQuery, type MentionOption } from "./mentions";
import type { UserRow } from "../../../lib/electric/types";

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

/** 空白のみ・空の下書きや入力無効時は送信しない。 */
export function canSubmitComposer(value: string, disabled: boolean): boolean {
  return !disabled && value.trim().length > 0;
}

/** キーボード操作を案内できないタッチ環境では送信ボタンを案内する。 */
export function composerSendHint(
  platform: ComposerPlatform,
  t: TFunction,
  isTouch = false,
): string {
  if (isTouch) {
    return t("composer.touch");
  }
  if (platform === "mac") {
    return t("composer.mac");
  }
  if (platform === "windows" || platform === "linux") {
    return t("composer.ctrl");
  }
  return t("composer.other");
}

interface ComposerProps {
  mentionUsers?: UserRow[];
  mentionMemberIds?: ReadonlySet<string>;
  /** 送信を実行し、成功したかを返す。false なら下書きを復元する。 */
  onTyping?: (active: boolean) => void;
  onSend: (content: string) => Promise<boolean>;
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
  mentionUsers = [],
  mentionMemberIds = new Set(),
  onSend,
  disabled = false,
  placeholder,
  ariaLabel,
}: ComposerProps) {
  const { t } = useTranslation();
  const resolvedPlaceholder = placeholder ?? t("messages.compose");
  const resolvedAriaLabel = ariaLabel ?? t("messages.compose");
  const [value, setValue] = useState("");
  const [caret, setCaret] = useState(0);
  const [activeOption, setActiveOption] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [composing, setComposing] = useState(false);
  const listId = useId();
  const range = !disabled && !dismissed && !composing ? mentionQuery(value, caret) : null;
  const options = range ? mentionOptions(mentionUsers, mentionMemberIds, range.query) : [];
  const selectedOption = Math.min(activeOption, Math.max(0, options.length - 1));
  function selectMention(option: MentionOption) {
    if (!range) return;
    const next = insertMention(value, range, option.name);
    setValue(next.value);
    setCaret(next.caret);
    setDismissed(true);
    onTyping?.(true);
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(next.caret, next.caret);
    });
  }
  const [platform] = useState(() => detectComposerPlatform());
  // タッチ端末には Ctrl / Cmd が無いため、キーボードショートカットを案内しない。
  const isTouch = useMediaQuery("(hover: none)");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useLayoutEffect(() => {
    const element = textareaRef.current;
    if (!element) {
      return;
    }
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, MAX_COMPOSER_HEIGHT)}px`;
    element.style.overflowY = element.scrollHeight > MAX_COMPOSER_HEIGHT ? "auto" : "hidden";
  }, [value]);

  async function submit() {
    const content = value.trim();
    if (!canSubmitComposer(value, disabled)) {
      return;
    }
    setValue("");
    onTyping?.(false);
    let sent = false;
    try {
      sent = await onSend(content);
    } catch {
      sent = false;
    }
    if (!sent) {
      // 失敗時は下書きを戻す。ただし待っている間に次の入力を始めていたら邪魔しない。
      setValue((current) => (current.length > 0 ? current : content));
    }
  }

  return (
    <form
      className="border-t px-3 py-3"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      {options.length > 0 ? (
        <ul
          id={listId}
          role="listbox"
          aria-label={t("mentions.suggestions")}
          className="mb-2 max-h-48 overflow-y-auto rounded-lg border bg-popover p-1"
        >
          {options.map((option, index) => (
            <li
              key={option.name}
              id={`${listId}-${index}`}
              role="option"
              aria-selected={index === selectedOption}
            >
              <button
                type="button"
                className={`flex w-full gap-2 rounded px-2 py-1 text-left text-sm ${index === selectedOption ? "bg-accent" : ""}`}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => selectMention(option)}
              >
                <span>@{option.name}</span>
                <span className="truncate text-muted-foreground">{option.label}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="flex items-end gap-2 rounded-xl border border-input bg-background p-1.5 transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/30 dark:bg-input/30">
        <Textarea
          ref={textareaRef}
          rows={1}
          value={value}
          disabled={disabled}
          placeholder={resolvedPlaceholder}
          aria-label={resolvedAriaLabel}
          className="min-h-9 resize-none border-0 bg-transparent px-2 py-1.5 text-sm shadow-none [field-sizing:fixed] focus-visible:ring-0 dark:bg-transparent"
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={options.length > 0}
          aria-controls={options.length ? listId : undefined}
          aria-activedescendant={options.length ? `${listId}-${selectedOption}` : undefined}
          onCompositionStart={() => setComposing(true)}
          onCompositionEnd={() => setComposing(false)}
          onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
          onChange={(event) => {
            setCaret(event.target.selectionStart);
            setActiveOption(0);
            setDismissed(false);
            setValue(event.target.value);
            onTyping?.(event.target.value.trim().length > 0);
          }}
          onBlur={() => {
            setDismissed(true);
            onTyping?.(false);
          }}
          onKeyDown={(event) => {
            if (
              options.length &&
              !event.nativeEvent.isComposing &&
              !event.metaKey &&
              !event.ctrlKey
            ) {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                setActiveOption(
                  (selectedOption + (event.key === "ArrowDown" ? 1 : -1) + options.length) %
                    options.length,
                );
                return;
              }
              if ((event.key === "Enter" && !event.shiftKey) || event.key === "Tab") {
                event.preventDefault();
                selectMention(options[selectedOption]!);
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
          size="icon-sm"
          disabled={!canSubmitComposer(value, disabled)}
          aria-label={t("messages.send")}
          title={t("messages.send")}
        >
          <SendIcon />
        </Button>
      </div>
      <p className="mt-1.5 px-1 text-[11px] text-muted-foreground">
        {composerSendHint(platform, t, isTouch)}
      </p>
    </form>
  );
}
