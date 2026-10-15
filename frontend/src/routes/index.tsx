import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({ component: Home });

function Home() {
  return (
    <div className="p-8">
      <h1 className="text-4xl font-bold">Zoo</h1>
      <p className="mt-4 text-lg">フロントエンドの足場（Phase 0）。画面は Phase 4 で実装します。</p>
    </div>
  );
}
