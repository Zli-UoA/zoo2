import { useEffect, useMemo, useState } from "react";
import { createConfiguredApiClient } from "../api/client";
import { useCollections } from "../electric/collections-context";
import type { AppConfig } from "../config";

export type AuthZAccount = { username: string; display_name: string; avatar_url: string | null };
const values = new WeakMap<AppConfig, AuthZAccount[]>();
const requests = new WeakMap<AppConfig, Promise<AuthZAccount[]>>();
export function useAuthZAccounts(): readonly AuthZAccount[] {
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [accounts, setAccounts] = useState<AuthZAccount[]>(values.get(config) ?? []);
  useEffect(() => {
    let active = true;
    const refresh = () => {
      let request = requests.get(config);
      if (!request) {
        request = client
          .get<{ accounts: AuthZAccount[] }>("/api/emoji-accounts")
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
  }, [client, config]);
  return accounts;
}
