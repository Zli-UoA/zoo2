import { Link } from "@tanstack/react-router";
import { ActivityIcon, HashIcon, type LucideIcon, PawPrintIcon } from "lucide-react";
import { cn } from "cn";

import { useUsers } from "../../hooks/useRealtime";
import { getCollections } from "../../lib/electric/collections";
import { ModeToggle } from "../theme/ModeToggle";
import { Avatar, AvatarFallback } from "../ui/avatar";
import { Separator } from "../ui/separator";
import { initials } from "../chat/format";

function RailLink({
  to,
  icon: Icon,
  label,
  exact = false,
}: {
  to: "/" | "/realtime";
  icon: LucideIcon;
  label: string;
  exact?: boolean;
}) {
  return (
    <Link
      to={to}
      activeOptions={{ exact }}
      aria-label={label}
      title={label}
      className="relative flex size-10 items-center justify-center rounded-xl text-muted-foreground transition-colors outline-none hover:bg-accent hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-[status=active]:bg-brand-muted data-[status=active]:text-brand-muted-foreground"
    >
      <Icon className="size-5" aria-hidden="true" />
    </Link>
  );
}

/**
 * 最左の固定レール。アプリの印と主要ナビ、テーマ切替、現在ユーザーを置く。
 * 画面幅に関わらず表示し、狭幅でも操作の入口を保つ。
 */
export function LeftRail() {
  const { config } = getCollections();
  const usersQuery = useUsers();
  const me = (usersQuery.data ?? []).find((user) => user.id === config.devUserId);
  const meLabel = me ? me.display_name || me.user_name : (config.devUserId ?? "?");

  return (
    <nav
      aria-label="メインナビゲーション"
      className={cn(
        "flex w-14 shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-3",
        "md:w-16",
      )}
    >
      <div className="mb-1 flex size-9 items-center justify-center rounded-xl bg-brand text-brand-foreground">
        <PawPrintIcon className="size-5" aria-hidden="true" />
        <span className="sr-only">Zoo</span>
      </div>
      <Separator className="my-1 w-6" />
      <RailLink to="/" icon={HashIcon} label="チャンネル" exact />
      {/* 同期の状態（/realtime）は開発時の確認用。本番ビルドでは出さない。 */}
      {import.meta.env.DEV ? (
        <RailLink to="/realtime" icon={ActivityIcon} label="同期の状態" exact />
      ) : null}

      <div className="mt-auto flex flex-col items-center gap-2">
        <ModeToggle />
        <Separator className="w-6" />
        <Avatar size="sm" title={meLabel}>
          <AvatarFallback>{initials(meLabel)}</AvatarFallback>
        </Avatar>
      </div>
    </nav>
  );
}
