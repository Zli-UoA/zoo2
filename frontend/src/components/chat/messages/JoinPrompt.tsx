import { Button } from "../../ui/button";

/**
 * 参加しないと書き込めない配置先（チャンネル・スレッド）で、入力欄の代わりに出す参加の案内。
 * `actionLabel` を省くと、説明だけを出す（操作できない理由を伝えるとき）。
 */
export function JoinPrompt({
  description,
  actionLabel,
  onAction,
  pending = false,
  disabled = false,
}: {
  description: string;
  actionLabel?: string;
  onAction?: () => void;
  pending?: boolean;
  disabled?: boolean;
}) {
  return (
    <div className="border-t px-3 py-3">
      <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed bg-muted/30 px-4 py-4 text-center">
        <p className="text-sm text-muted-foreground">{description}</p>
        {actionLabel ? (
          <Button
            size="sm"
            onClick={onAction}
            disabled={disabled || pending || !onAction}
            aria-busy={pending}
          >
            {actionLabel}
          </Button>
        ) : null}
      </div>
    </div>
  );
}
