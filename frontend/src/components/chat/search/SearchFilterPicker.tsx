import { Popover } from "@base-ui/react/popover";
import { ChevronDownIcon, SearchIcon, type LucideIcon } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export interface SearchFilterOption {
  id: string;
  label: string;
  /** 名前のほかに照合する文字列（`@user_name` など）。 */
  keywords?: string;
  /** 名前の横に薄く出す補足。 */
  detail?: string;
  /** 行の先頭のアイコン（アバターやチャンネルの印）。 */
  icon: ReactNode;
}

/** 一覧に一度に出す数。多いときは検索欄で絞ってもらう。 */
const SHOWN = 50;

/**
 * 全文検索の「送信者」「チャンネル」の絞り込み。押すと検索欄と、アイコン付きのチェックボックスの一覧が開き、
 * 複数を選べる（どれかに当たる結果）。選んだものは一覧の先頭に出す。`max` 個まで。
 */
export function SearchFilterPicker({
  label,
  icon: Icon,
  options,
  selected,
  onChange,
  searchLabel,
  max,
}: {
  label: string;
  icon: LucideIcon;
  options: readonly SearchFilterOption[];
  selected: readonly string[];
  onChange: (ids: string[]) => void;
  searchLabel: string;
  max: number;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const chosen = useMemo(() => new Set(selected), [selected]);
  const shown = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    const matches = options.filter(
      (option) =>
        !needle ||
        option.label.toLocaleLowerCase().includes(needle) ||
        !!option.keywords?.toLocaleLowerCase().includes(needle),
    );
    // 選んだものを先に。
    return [
      ...matches.filter((option) => chosen.has(option.id)),
      ...matches.filter((option) => !chosen.has(option.id)),
    ].slice(0, SHOWN);
  }, [options, query, chosen]);
  const first = options.find((option) => option.id === selected[0]);
  const summary =
    selected.length === 0
      ? label
      : selected.length === 1 && first
        ? `${label}: ${first.label}`
        : t("globalSearch.filters.selectedCount", { label, count: selected.length });
  const full = selected.length >= max;
  const toggle = (id: string, on: boolean) =>
    onChange(on ? [...selected, id] : selected.filter((value) => value !== id));

  return (
    <Popover.Root onOpenChange={(open) => !open && setQuery("")}>
      <Popover.Trigger
        render={
          <Button
            type="button"
            variant={selected.length ? "default" : "outline"}
            className="max-w-56"
            aria-label={summary}
          />
        }
      >
        <Icon aria-hidden="true" />
        <span className="min-w-0 truncate">{summary}</span>
        <ChevronDownIcon aria-hidden="true" className="opacity-60" />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Positioner side="bottom" align="start" sideOffset={6} className="z-50">
          <Popover.Popup className="w-[min(20rem,calc(100vw-2rem))] rounded-xl border bg-popover p-3 text-popover-foreground shadow-xl outline-none">
            <Popover.Title className="sr-only">{label}</Popover.Title>
            <div className="relative">
              <SearchIcon
                aria-hidden="true"
                className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                type="search"
                autoFocus
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={searchLabel}
                aria-label={searchLabel}
                className="pl-8"
              />
            </div>
            {full ? (
              <p className="mt-2 text-xs text-muted-foreground">
                {t("globalSearch.filters.max", { count: max })}
              </p>
            ) : null}
            {shown.length ? (
              <ul className="mt-2 max-h-72 space-y-0.5 overflow-y-auto" aria-label={label}>
                {shown.map((option) => {
                  const checked = chosen.has(option.id);
                  return (
                    <li key={option.id}>
                      <label className="flex cursor-pointer items-center gap-2 rounded-md px-1.5 py-1.5 hover:bg-muted has-disabled:cursor-not-allowed has-disabled:opacity-50">
                        <input
                          type="checkbox"
                          className="size-4 shrink-0 accent-primary"
                          checked={checked}
                          disabled={!checked && full}
                          onChange={(event) => toggle(option.id, event.target.checked)}
                        />
                        <span className="flex size-6 shrink-0 items-center justify-center">
                          {option.icon}
                        </span>
                        <span className="min-w-0 truncate text-sm">{option.label}</span>
                        {option.detail ? (
                          <span className="min-w-0 shrink truncate text-xs text-muted-foreground">
                            {option.detail}
                          </span>
                        ) : null}
                      </label>
                    </li>
                  );
                })}
              </ul>
            ) : (
              <p className="mt-3 px-1.5 text-sm text-muted-foreground">
                {t("globalSearch.filters.noMatches")}
              </p>
            )}
            {selected.length ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="mt-2 w-full"
                onClick={() => onChange([])}
              >
                {t("globalSearch.filters.clearSelection")}
              </Button>
            ) : null}
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
