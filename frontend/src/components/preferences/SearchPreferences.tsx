import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { createConfiguredApiClient, type ApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { isSearchEngine, type SearchEngine } from "../../lib/searchEngine";

type Preferences = { search_engine: SearchEngine };
type State = {
  engine: SearchEngine;
  loading: boolean;
  saving: boolean;
  error: "load" | "save" | null;
  save: (engine: SearchEngine) => Promise<void>;
  retry: () => void;
};
const SearchPreferencesContext = createContext<State | null>(null);
export function useSearchEngine(): SearchEngine {
  return useContext(SearchPreferencesContext)?.engine ?? "google";
}
export function useSearchPreferences() {
  return useContext(SearchPreferencesContext);
}
export function SearchPreferencesProvider({ children }: { children: ReactNode }) {
  const { currentUserId } = useCurrentUser();
  const { config } = useCollections();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  return (
    <PersonalSearchPreferences key={currentUserId} client={client} enabled={!!currentUserId}>
      {children}
    </PersonalSearchPreferences>
  );
}

// Remount on account changes, aborting requests and discarding the previous user's settings.
export function PersonalSearchPreferences({
  client,
  enabled,
  children,
}: {
  client: ApiClient;
  enabled: boolean;
  children: ReactNode;
}) {
  const [engine, setEngine] = useState<SearchEngine>("google");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<State["error"]>(null);
  const [attempt, setAttempt] = useState(0);
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    return () => controller.abort();
  }, []);
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    void client
      .get<Preferences>("/api/users/me/preferences", { signal: controller.signal })
      .then((value) => {
        if (controller.signal.aborted) return;
        if (!isSearchEngine(value.search_engine)) throw new Error("Invalid search engine");
        setEngine(value.search_engine);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError("load");
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [client, enabled, attempt]);
  async function save(next: SearchEngine) {
    if (!enabled || loading || saving || error === "load" || next === engine) return;
    setSaving(true);
    setError(null);
    const controller = lifetime.current;
    try {
      await client.patch<Preferences>(
        "/api/users/me/preferences",
        { search_engine: next },
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) setEngine(next);
    } catch {
      if (!controller.signal.aborted) setError("save");
    } finally {
      if (!controller.signal.aborted) setSaving(false);
    }
  }
  return (
    <SearchPreferencesContext.Provider
      value={{ engine, loading, saving, error, save, retry: () => setAttempt((n) => n + 1) }}
    >
      {children}
    </SearchPreferencesContext.Provider>
  );
}
