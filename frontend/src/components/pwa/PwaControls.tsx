import { createContext, useContext, useEffect, useId, useState, type ReactNode } from "react";
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
import { Select } from "../ui/select";
import { Button } from "../ui/button";

type InstallEvent = Event & {
  prompt(): Promise<void>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
};

const PwaContext = createContext<{
  event: InstallEvent | null;
  clear: () => void;
  push: ReturnType<typeof usePushControls>;
} | null>(null);
export function PwaProvider({ children }: { children: ReactNode }) {
  const [event, setEvent] = useState<InstallEvent | null>(null);
  const push = usePushControls();
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
    <PwaContext.Provider value={{ event, clear: () => setEvent(null), push }}>
      {children}
    </PwaContext.Provider>
  );
}

function usePushControls() {
  const { t } = useTranslation();
  const { config } = useCollections();
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
  return { label, enabled, busy, supported, publicKey, blocked, toggle };
}

export function PwaControls({ presentation = "icon" }: { presentation?: "icon" | "select" }) {
  const context = useContext(PwaContext);
  return context ? (
    <PwaControlButtons push={context.push} prompt={context} presentation={presentation} />
  ) : (
    <StandalonePwaControls presentation={presentation} />
  );
}

function StandalonePwaControls({ presentation }: { presentation: "icon" | "select" }) {
  const push = usePushControls();
  return <PwaControlButtons push={push} prompt={null} presentation={presentation} />;
}

function PwaControlButtons({
  push,
  prompt,
  presentation,
}: {
  presentation: "icon" | "select";
  push: ReturnType<typeof usePushControls>;
  prompt: { event: InstallEvent | null; clear: () => void } | null;
}) {
  const { t } = useTranslation();
  const { label, enabled, busy, supported, publicKey, blocked, toggle } = push;
  const install = prompt?.event ?? null;
  const installButton = install ? (
    <Button
      variant={presentation === "select" ? "outline" : "ghost"}
      size={presentation === "select" ? "sm" : "icon"}
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
      {presentation === "select" ? t("pwa.install") : null}
    </Button>
  ) : null;
  return (
    <div
      className={
        presentation === "select" ? "flex w-40 flex-col items-end gap-2" : "flex items-center gap-2"
      }
    >
      {presentation === "icon" ? installButton : null}
      {presentation === "select" ? (
        <PushNotificationSelect push={push} />
      ) : (
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
      )}
      {presentation === "select" ? installButton : null}
    </div>
  );
}

export function PushNotificationSelect({ push }: { push: ReturnType<typeof usePushControls> }) {
  const { t } = useTranslation();
  const descriptionId = useId();
  const unavailable = !push.supported || (!push.enabled && (!push.publicKey || push.blocked));
  return (
    <div className="w-40 space-y-1.5">
      <Select
        aria-label={t("pwa.title")}
        aria-describedby={unavailable ? descriptionId : undefined}
        aria-busy={push.busy}
        disabled={push.busy || unavailable}
        value={unavailable ? "unavailable" : push.enabled ? "on" : "off"}
        onChange={(event) => {
          if ((event.target.value === "on") !== push.enabled) void push.toggle();
        }}
      >
        {unavailable ? <option value="unavailable">{t("pwa.unavailable")}</option> : null}
        <option value="on">{t("pwa.on")}</option>
        <option value="off">{t("pwa.off")}</option>
      </Select>
      {unavailable ? (
        <p id={descriptionId} className="text-xs text-muted-foreground">
          {push.label}
        </p>
      ) : null}
    </div>
  );
}
