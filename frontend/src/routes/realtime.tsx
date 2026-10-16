import { Link, createFileRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { SyncStatus } from "../components/SyncStatus";

export const Route = createFileRoute("/realtime")({ component: RealtimeRoute });

/**
 * Phase 2/3 の検証 UI はトップとチャンネル画面へ移行した。ここは同期の接続状態を
 * 確認するための最小画面として残し、実ルートへは `Link` で遷移する。
 */
function RealtimeRoute() {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    setMounted(true);
  }, []);

  return (
    <main className="mx-auto max-w-4xl p-8">
      <h1 className="text-2xl font-bold">同期の状態</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        Electric Shape の購読と REST 書き込みの検証 UI はトップとチャンネル画面へ移行しました。
        ここでは接続設定だけを表示します。
      </p>
      <div className="mt-4">
        {mounted ? (
          <SyncStatus />
        ) : (
          <p className="text-sm text-muted-foreground">ブラウザで読み込み中…</p>
        )}
      </div>
      <div className="mt-4">
        <Link to="/" className="text-sm underline underline-offset-4">
          チャンネル一覧へ
        </Link>
      </div>
    </main>
  );
}
