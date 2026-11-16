import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { createConfiguredApiClient } from "../../lib/api/client";
import { subscribeAuthSessionEnded } from "../../lib/auth";
import type { AppConfig } from "../../lib/config";
import { SipController, initialSipState } from "../../lib/sip/controller";
import { createSipDriver } from "../../lib/sip/driver";
import type { SipDirectory, SipLineInput } from "../../lib/sip/types";
import { useCurrentUser } from "../../hooks/useCurrentUser";
import { startRingtone } from "../../lib/call/call-sounds";
import { useCall } from "../../lib/call/call-context";

interface SipContextValue {
  controller: SipController;
  state: typeof initialSipState;
  directory: SipDirectory;
  loading: boolean;
  loadError: boolean;
  loaded: boolean;
  channelBusy: boolean;
  refresh(this: void): Promise<void>;
  save(this: void, id: string | null, input: SipLineInput): Promise<void>;
  remove(this: void, id: string): Promise<void>;
}
const SipContext = createContext<SipContextValue | null>(null);
export function SipProvider({ config, children }: { config: AppConfig; children: ReactNode }) {
  const api = useMemo(() => createConfiguredApiClient(config), [config]);
  const controller = useMemo(() => new SipController(api, createSipDriver), [api]);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.snapshot,
    controller.snapshot,
  );
  const { currentUserId } = useCurrentUser();
  const { currentChannelId } = useCall();
  const [directory, setDirectory] = useState<SipDirectory>({ enabled: false, lines: [] });
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const epoch = useRef(0);
  const flight = useRef<Promise<void> | null>(null);
  const abort = useRef<AbortController | null>(null);
  const refresh = useCallback(() => {
    if (flight.current) return flight.current;
    const generation = epoch.current;
    const request = new AbortController();
    abort.current = request;
    setLoading(true);
    setLoadError(false);
    const promise = api
      .get<SipDirectory>("/api/sip/lines", { signal: request.signal })
      .then((value) => {
        if (generation !== epoch.current) return;
        setDirectory(value);
        setLoaded(true);
        const lineId = controller.snapshot().lineId;
        if (lineId && !value.lines.some((line) => line.id === lineId && line.can_connect))
          controller.disconnect();
      })
      .catch(() => {
        if (generation === epoch.current) setLoadError(true);
      })
      .finally(() => {
        if (generation === epoch.current) {
          setLoading(false);
          flight.current = null;
        }
      });
    flight.current = promise;
    return promise;
  }, [api, controller]);
  useEffect(() => {
    const reset = () => {
      epoch.current++;
      abort.current?.abort();
      flight.current = null;
      controller.disconnect();
      setDirectory({ enabled: false, lines: [] });
      setLoaded(false);
      setLoading(false);
      setLoadError(false);
    };
    reset();
    const unsubscribe = subscribeAuthSessionEnded(() => {
      reset();
    });
    return () => {
      unsubscribe();
      epoch.current++;
      abort.current?.abort();
      flight.current = null;
      controller.disconnect();
    };
  }, [controller, currentUserId]);
  // A single microphone session is allowed. Starting a channel call ends SIP registration/calls.
  useEffect(() => {
    if (currentChannelId) controller.disconnect();
  }, [controller, currentChannelId]);
  useEffect(() => {
    if (state.status === "incoming") return startRingtone();
  }, [state.status]);
  const value: SipContextValue = {
    controller,
    state,
    directory,
    loading,
    loaded,
    loadError,
    channelBusy: !!currentChannelId,
    refresh,
    save: async (id, input) => {
      const generation = epoch.current;
      await api.post(id ? `/api/sip/lines/${encodeURIComponent(id)}` : "/api/sip/lines", input);
      if (generation !== epoch.current) return;
      if (controller.snapshot().lineId === id) controller.disconnect();
      // A directory read begun before the mutation may still be in flight.
      // Let it settle, then coalesce one fresh read of the committed settings.
      if (flight.current) await flight.current;
      if (generation === epoch.current) await refresh();
    },
    remove: async (id) => {
      const generation = epoch.current;
      await api.delete(`/api/sip/lines/${encodeURIComponent(id)}`);
      if (generation !== epoch.current) return;
      if (controller.snapshot().lineId === id) controller.disconnect();
      // A directory read begun before the mutation may still be in flight.
      // Let it settle, then coalesce one fresh read of the committed settings.
      if (flight.current) await flight.current;
      if (generation === epoch.current) await refresh();
    },
  };
  return <SipContext.Provider value={value}>{children}</SipContext.Provider>;
}
export function useSip() {
  return useContext(SipContext);
}
