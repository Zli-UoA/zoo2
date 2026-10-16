import { useLayoutEffect, useRef, useState } from "react";
import { SendIcon } from "lucide-react";

import { Button } from "../../ui/button";
import { Textarea } from "../../ui/textarea";

const MAX_COMPOSER_HEIGHT = 200;

interface ComposerProps {
  /** 送信を実行し、成功したかを返す。false なら下書きを復元する。 */
  onSend: (content: string) => Promise<boolean>;
  disabled?: boolean;
}

/**
 * メッセージ入力。複数行 autosize、Enter 送信 / Shift+Enter 改行。
 * 送信は楽観更新に任せ、入力欄は即座にクリアする。確定に失敗したら下書きを戻す。
 */
export function Composer({ onSend, disabled = false }: ComposerProps) {
  const [value, setValue] = useState("");
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
    if (!content || disabled) {
      return;
    }
    setValue("");
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
      <div className="flex items-end gap-2 rounded-xl border border-input bg-background p-1.5 transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/30 dark:bg-input/30">
        <Textarea
          ref={textareaRef}
          rows={1}
          value={value}
          disabled={disabled}
          placeholder="メッセージを送信"
          aria-label="メッセージを送信"
          className="min-h-9 resize-none border-0 bg-transparent px-2 py-1.5 text-sm shadow-none [field-sizing:fixed] focus-visible:ring-0 dark:bg-transparent"
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void submit();
            }
          }}
        />
        <Button
          type="submit"
          size="icon-sm"
          disabled={disabled || !value.trim()}
          aria-label="送信"
          title="送信"
        >
          <SendIcon />
        </Button>
      </div>
      <p className="mt-1.5 px-1 text-[11px] text-muted-foreground">
        Enter で送信 / Shift+Enter で改行
      </p>
    </form>
  );
}
