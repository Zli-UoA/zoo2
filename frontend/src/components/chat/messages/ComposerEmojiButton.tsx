import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import { SmileIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../ui/button";
import { ReactionPicker } from "./ReactionPicker";

/** Keep the original selection while the shared picker owns focus. */
export function ComposerEmojiButton({
  textareaRef,
  value,
  onChange,
  disabled,
  isComposing,
}: {
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  isComposing?: () => boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const selection = useRef({ start: 0, end: 0 });
  const caret = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (open || caret.current === null) return;
    textareaRef.current?.focus();
    textareaRef.current?.setSelectionRange(caret.current, caret.current);
    caret.current = null;
  }, [open, textareaRef]);
  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="icon-lg"
        className="md:mt-1"
        disabled={disabled}
        aria-label={t("reactions.tabEmoji")}
        title={t("reactions.tabEmoji")}
        onClick={() => {
          if (isComposing?.()) return;
          const input = textareaRef.current;
          selection.current = {
            start: input?.selectionStart ?? value.length,
            end: input?.selectionEnd ?? value.length,
          };
          setOpen(true);
        }}
      >
        <SmileIcon />
      </Button>
      <ReactionPicker
        open={open}
        onOpenChange={setOpen}
        purpose="message"
        onSelect={(emoji) => {
          if (disabled || isComposing?.()) return;
          const { start, end } = selection.current;
          onChange(value.slice(0, start) + emoji + value.slice(end));
          caret.current = start + emoji.length;
        }}
      />
    </>
  );
}
