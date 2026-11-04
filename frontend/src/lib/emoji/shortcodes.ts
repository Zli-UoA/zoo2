import { useEffect, useMemo, useState } from "react";
import { createConfiguredApiClient } from "../api/client";
import { resolvePublicConfig, type AppConfig } from "../config";
import { useOptionalCollections } from "../electric/collections-context";
import fallback from "./shortcodes.json";
const values = new WeakMap<AppConfig, Record<string, string>>();
const requests = new WeakMap<AppConfig, Promise<Record<string, string>>>();
const checked = new WeakMap<AppConfig, number>();
export function useShortcodes(enabled = true): Readonly<Record<string, string>> {
  const collections = useOptionalCollections();
  const config = useMemo(() => collections?.config ?? resolvePublicConfig(), [collections]);
  const [value, setValue] = useState<Record<string, string>>(() => values.get(config) ?? fallback);
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    const refresh = () => {
      if (Date.now() - (checked.get(config) ?? 0) < 3_600_000) {
        setValue(values.get(config) ?? fallback);
        return;
      }
      let request = requests.get(config);
      if (!request) {
        request = createConfiguredApiClient(config)
          .get<{ shortcodes: Record<string, string> }>("/api/emoji-shortcodes")
          .then(({ shortcodes }) => {
            if (Object.keys(shortcodes).length) values.set(config, shortcodes);
            checked.set(config, Date.now());
            return values.get(config) ?? fallback;
          })
          .finally(() => requests.delete(config));
        requests.set(config, request);
      }
      void request
        .then((next) => {
          if (active) setValue(next);
        })
        .catch(() => {});
    };
    refresh();
    const timer = window.setInterval(refresh, 3_600_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [config, enabled]);
  return values.get(config) ?? value;
}
