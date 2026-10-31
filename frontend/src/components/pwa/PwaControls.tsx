import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { BellIcon, BellOffIcon, DownloadIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ApiError, createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import {
  applicationServerKey,
  disablePush,
  registerWorker,
  saveSubscription,
  supportsPush,
} from "../../lib/pwa/push";
import { Button } from "../ui/button";

type InstallEvent = Event & {
  prompt(): Promise<void>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
};

const InstallPromptContext = createContext<{
  event: InstallEvent | null;
  clear: () => void;
} | null>(null);
export function InstallPromptProvider({ children }: { children: ReactNode }) {
  const [event, setEvent] = useState<InstallEvent | null>(null);
  useEffect(() => {
    const onInstall = (next: Event) => {
      next.preventDefault();
      setEvent(next as InstallEvent);
    };
    const installed = () => setEvent(null);
    window.addEventListener("beforeinstallprompt", onInstall);
    window.addEventListener("appinstalled", installed);
    return () => {
      window.removeEventListener("beforeinstallprompt", onInstall);
      window.removeEventListener("appinstalled", installed);
    };
  }, []);
  return (
    <InstallPromptContext.Provider value={{ event, clear: () => setEvent(null) }}>
      {children}
    </InstallPromptContext.Provider>
  );
}

export function PwaControls() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const prompt = useContext(InstallPromptContext);
  const install = prompt?.event ?? null;
  const [supported, setSupported] = useState(false);
  const [enabled, setEnabled] = useState(false);
  const [publicKey, setPublicKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [blocked, setBlocked] = useState(false);

  useEffect(() => {
    let cancelled = false;
    if (window.isSecureContext && "serviceWorker" in navigator) {
      void registerWorker()
        .then(async (worker) => {
          if (!supportsPush()) return;
          const [existing, settings] = await Promise.all([
            worker.pushManager.getSubscription(),
            createConfiguredApiClient(config).get<{ publicKey: string | null }>("/api/push/config"),
          ]);
          let subscription = existing;
          if (subscription && settings.publicKey) {
            try {
              await saveSubscription(config, subscription);
            } catch (error) {
              if (!(error instanceof ApiError) || error.status !== 409) throw error;
              // Another account previously owned this browser's endpoint. Require a fresh opt-in.
              if (!(await subscription.unsubscribe()))
                throw new Error("Could not revoke push subscription");
              subscription = null;
            }
          }
          if (!cancelled) {
            setSupported(true);
            setPublicKey(settings.publicKey);
            setEnabled(Boolean(subscription));
            setBlocked(Notification.permission === "denied");
          }
        })
        .catch(() => {
          /* PWA setup must not interrupt chat. The control remains unavailable. */
        });
    }
    return () => {
      cancelled = true;
    };
  }, [config]);

  const toggle = async () => {
    setBusy(true);
    try {
      if (enabled) {
        await disablePush(config);
        setEnabled(false);
      } else {
        // Keep the permission request in the click gesture (required by Safari).
        const permission = await Notification.requestPermission();
        if (permission !== "granted") {
          setBlocked(permission === "denied");
          toast.error(t("pwa.permission"));
          return;
        }
        const worker = await registerWorker();
        await navigator.serviceWorker.ready;
        let subscription = await worker.pushManager.getSubscription();
        const created = !subscription;
        subscription ??= await worker.pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: applicationServerKey(publicKey!),
        });
        try {
          await saveSubscription(config, subscription);
        } catch (error) {
          if (created) await subscription.unsubscribe();
          throw error;
        }
        setEnabled(true);
      }
    } catch {
      const worker = await navigator.serviceWorker.getRegistration("/").catch(() => undefined);
      const subscription = await worker?.pushManager.getSubscription().catch(() => null);
      setEnabled(Boolean(subscription));
      toast.error(t("pwa.error"));
    } finally {
      setBusy(false);
    }
  };

  const label = !supported
    ? t("pwa.unsupported")
    : !publicKey
      ? t("pwa.unconfigured")
      : blocked
        ? t("pwa.blocked")
        : enabled
          ? t("pwa.disable")
          : t("pwa.enable");
  return (
    <>
      {install ? (
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("pwa.install")}
          title={t("pwa.install")}
          onClick={() => {
            void install
              .prompt()
              .then(() => install.userChoice)
              .then(() => prompt?.clear())
              .catch(() => toast.error(t("pwa.error")));
          }}
        >
          <DownloadIcon aria-hidden="true" />
        </Button>
      ) : null}
      <Button
        variant="ghost"
        size="icon"
        aria-label={label}
        title={label}
        aria-pressed={enabled}
        disabled={busy || !supported || (!enabled && (!publicKey || blocked))}
        onClick={() => {
          void toggle();
        }}
      >
        {enabled ? <BellIcon aria-hidden="true" /> : <BellOffIcon aria-hidden="true" />}
      </Button>
    </>
  );
}
