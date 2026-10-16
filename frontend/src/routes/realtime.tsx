import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { RealtimeReadView } from "../components/RealtimeReadView";

export const Route = createFileRoute("/realtime")({ component: RealtimeRoute });

function RealtimeRoute() {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    setMounted(true);
  }, []);

  return (
    <main className="mx-auto max-w-4xl p-8">
      {mounted ? (
        <RealtimeReadView />
      ) : (
        <p className="text-sm text-muted-foreground">ブラウザで読み込み中…</p>
      )}
    </main>
  );
}
