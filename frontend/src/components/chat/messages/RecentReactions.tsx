import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { kitchenLabel, type KitchenCatalog } from "../../../lib/emoji/kitchen";
import { useWorkspaceEmojis } from "../../../lib/emoji/workspace";
import { loadRecentEmojis } from "../../../lib/emoji/recents";
import { useKitchenCatalog } from "../../../hooks/useKitchenCatalog";
import { ReactionEmoji } from "./ReactionEmoji";

/**
 * クイックリアクションに出す最近使った絵文字。
 * ワークスペース外のカスタム絵文字は送信できないので、現在のカタログで解決できるものだけを残す。
 */
export function RecentReactions({
  catalog,
  onSelect,
}: {
  catalog?: KitchenCatalog | null;
  onSelect: (value: string) => void;
}) {
  const { t } = useTranslation();
  const [values] = useState(() => loadRecentEmojis());
  const { emojis } = useWorkspaceEmojis();
  const available = useMemo(
    () =>
      values
        .filter((value) => {
          // チャットでは ActivityPub の画像付きリアクションは送れない。
          if (value.startsWith("apub:")) return false;
          // アカウントのショートコードは MfmAccounts 側で解決するのでカタログ照合しない。
          if (value.startsWith(":@")) return true;
          if (!value.startsWith(":")) return true;
          const name = value.slice(1, -1);
          return emojis.some((emoji) => emoji.name === name || emoji.aliases?.includes(name));
        })
        .slice(0, 6),
    [values, emojis],
  );

  const kitchen = useKitchenCatalog(
    !catalog && available.some((value) => value.startsWith("kitchen:")),
  );

  if (available.length === 0) return null;
  return (
    <div role="group" aria-label={t("reactions.recent")}>
      <p className="mb-2 text-xs text-muted-foreground">{t("reactions.recent")}</p>
      <div className="flex flex-wrap gap-1">
        {available.map((emoji) => (
          <button
            key={emoji}
            type="button"
            aria-label={kitchenLabel(emoji)}
            className="flex size-11 items-center justify-center rounded-lg outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
            onClick={() => onSelect(emoji)}
          >
            <ReactionEmoji emoji={emoji} catalog={catalog ?? kitchen.catalog} size={26} />
          </button>
        ))}
      </div>
    </div>
  );
}
