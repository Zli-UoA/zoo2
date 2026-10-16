import { Link, createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({ component: Home });

function Home() {
  return (
    <div className="p-8">
      <h1 className="text-4xl font-bold">Zoo</h1>
      <p className="mt-4 text-lg">フロントエンドの足場（Phase 0）。画面は Phase 4 で実装します。</p>
      <Link className="mt-4 inline-block underline" to="/realtime">
        Phase 2/3 の確認用ページ（リアルタイム読み取りと書き込み）
      </Link>
    </div>
  );
}
