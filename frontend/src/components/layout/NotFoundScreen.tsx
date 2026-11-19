import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { ArrowLeftIcon } from "lucide-react";

import { cn } from "cn";
import { buttonVariants } from "../ui/button";

/**
 * 未知のルートや notFound 例外で出す 404 画面。
 *
 * `createRootRoute` の既定の `<p>Not Found</p>` を置き換える。シェル配下の
 * catch-all からはアプリシェル内の本文として、ルーターの
 * `defaultNotFoundComponent` からはシェル外のフォールバックとして描画する。
 */
export function NotFoundScreen({ className }: { className?: string }) {
  const { t } = useTranslation();

  return (
    <div className={cn("flex h-full min-h-0 flex-col items-center justify-center p-6", className)}>
      <div className="flex max-w-sm flex-col items-center gap-3 text-center">
        <span aria-hidden="true" className="text-4xl font-bold text-muted-foreground">
          404
        </span>
        <h1 className="text-lg font-semibold">{t("notFoundPage.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("notFoundPage.description")}</p>
        <Link to="/" className={buttonVariants({ variant: "outline", size: "sm" })}>
          <ArrowLeftIcon />
          {t("channels.back")}
        </Link>
      </div>
    </div>
  );
}
