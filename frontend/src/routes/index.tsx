import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { ChannelDirectory } from "../components/ChannelDirectory";

export const Route = createFileRoute("/")({ component: Home });

function Home() {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    setMounted(true);
  }, []);

  return (
    <main className="mx-auto max-w-4xl p-8">
      {mounted ? (
        <ChannelDirectory />
      ) : (
        <p className="text-sm text-muted-foreground">ブラウザで読み込み中…</p>
      )}
    </main>
  );
}
