import { useEffect, useState } from "react";

import { loadKitchenCatalog, type KitchenCatalog } from "../lib/emoji/kitchen";

export function useKitchenCatalog(enabled: boolean) {
  const [catalog, setCatalog] = useState<KitchenCatalog | null>(null);
  const [error, setError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    setError(false);
    void loadKitchenCatalog().then(
      (value) => {
        if (active) setCatalog(value);
      },
      () => {
        if (active) setError(true);
      },
    );
    return () => {
      active = false;
    };
  }, [enabled, attempt]);
  return { catalog, error, retry: () => setAttempt((value) => value + 1) };
}
