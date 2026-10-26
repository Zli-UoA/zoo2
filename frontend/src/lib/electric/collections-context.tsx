import { createContext, useContext, type ReactNode } from "react";

import { getCollections, type CollectionsBundle } from "./collections";

const CollectionsContext = createContext<CollectionsBundle | null>(null);

/**
 * アプリの実コレクションを差し替えるための Provider。
 * Storybook やテストで fixture のコレクションを注入する。
 */
export function CollectionsProvider({
  value,
  children,
}: {
  value: CollectionsBundle;
  children: ReactNode;
}) {
  return <CollectionsContext.Provider value={value}>{children}</CollectionsContext.Provider>;
}

/**
 * Provider があればそのバンドルを、なければ従来どおり `getCollections()` を返す。
 * 既定経路はシングルトンをそのまま使うため、アプリの挙動は変わらない。
 */
export function useCollections(): CollectionsBundle {
  return useContext(CollectionsContext) ?? getCollections();
}

/** Provider が無いときに `getCollections()` の解決を呼び出し側へ遅らせる。 */
export function useOptionalCollections(): CollectionsBundle | null {
  return useContext(CollectionsContext);
}
