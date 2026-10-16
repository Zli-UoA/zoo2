import { HashIcon } from "lucide-react";

/** チャンネル未選択のときのメイン面。 */
export function WelcomePanel() {
  return (
    <div className="flex h-full flex-1 items-center justify-center p-6">
      <div className="flex max-w-md flex-col items-center gap-3 text-center">
        <div className="flex size-12 items-center justify-center rounded-2xl bg-brand text-brand-foreground">
          <HashIcon className="size-6" aria-hidden="true" />
        </div>
        <h1 className="font-heading text-xl font-semibold tracking-tight">ようこそ Zoo へ</h1>
        <p className="text-sm text-muted-foreground">
          左のリストからチャンネルを選択すると、メッセージのやり取りを表示します。
        </p>
      </div>
    </div>
  );
}
