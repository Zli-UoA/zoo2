import { useEffect, useState } from "react";
import { createConfiguredApiClient } from "../api/client";
import { useOptionalCollections } from "../electric/collections-context";
import { getCollections } from "../electric/collections";
import type { AppConfig } from "../config";

export type AuthZAccount = { username: string; display_name: string; avatar_url: string | null };
const values = new WeakMap<AppConfig, AuthZAccount[]>();
const requests = new WeakMap<AppConfig, Promise<AuthZAccount[]>>();
export function useAuthZAccounts(): readonly AuthZAccount[] {
  const collections = useOptionalCollections();
  const [accounts, setAccounts] = useState<AuthZAccount[]>(() =>
    collections ? (values.get(collections.config) ?? []) : [],
  );
  useEffect(() => {
    let active = true;
    let config: AppConfig;
    try {
      config = (collections ?? getCollections()).config;
    } catch {
      setAccounts([]);
      return;
    }
    const refresh = () => {
      let request = requests.get(config);
      if (!request) {
        request = Promise.resolve()
          .then(() =>
            createConfiguredApiClient(config).get<{ accounts: AuthZAccount[] }>(
              "/api/emoji-accounts",
            ),
          )
          .then((result) => result.accounts)
          .catch(() => [])
          .then((result) => {
            values.set(config, result);
            return result;
          })
          .finally(() => requests.delete(config));
        requests.set(config, request);
      }
      void request.then((result) => {
        if (active) setAccounts(result);
      });
    };
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [collections]);
  return accounts;
}
