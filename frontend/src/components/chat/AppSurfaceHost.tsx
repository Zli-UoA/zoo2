import { useCurrentUser } from "../../hooks/useCurrentUser";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { createConfiguredApiClient } from "../../lib/api/client";
import { useCollections } from "../../lib/electric/collections-context";
import { useShell } from "../layout/shell-context";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import { AppBlocks } from "./AppBlocks";

type Surface = {
  id: string;
  kind: string;
  app_id: string;
  name: string;
  channel_id: string | null;
  document: { blocks?: unknown };
};
export function AppSurfaceHost() {
  const { currentUserId } = useCurrentUser();
  return currentUserId ? <UserSurfaces key={currentUserId} /> : null;
}
function UserSurfaces() {
  const { t } = useTranslation();
  const { config } = useCollections();
  const shell = useShell();
  const client = useMemo(() => createConfiguredApiClient(config), [config]);
  const [surfaces, setSurfaces] = useState<Surface[]>([]);
  useEffect(() => {
    const controller = new AbortController();
    const load = () => {
      if (!document.hidden)
        void client
          .get<Surface[]>("/api/app-surfaces", { signal: controller.signal })
          .then(setSurfaces)
          .catch(() => {});
    };
    load();
    const timer = setInterval(load, 2000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [client]);
  function remove(id: string) {
    setSurfaces((rows) => rows.filter((s) => s.id !== id));
  }
  async function dismiss(id: string) {
    try {
      await client.request(`/api/app-surfaces/${id}`, { method: "DELETE" });
      remove(id);
    } catch {
      /* Retry on the next interaction. */
    }
  }
  const modal = surfaces.find((surface) => surface.kind === "modal");
  return (
    <>
      {modal ? (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) void dismiss(modal.id);
          }}
        >
          <DialogContent className="max-h-[85dvh] overflow-y-auto">
            <DialogHeader>
              <DialogTitle>{modal.name}</DialogTitle>
              <DialogDescription>{t("apps.form")}</DialogDescription>
            </DialogHeader>
            <AppBlocks
              key={modal.id}
              appId={modal.app_id}
              surfaceId={modal.id}
              channelId={modal.channel_id}
              blocks={modal.document.blocks}
              submit
              onDone={() => remove(modal.id)}
            />
          </DialogContent>
        </Dialog>
      ) : null}
      <aside
        className="fixed bottom-4 right-4 z-30 max-h-[40dvh] w-[min(24rem,calc(100vw-2rem))] space-y-2 overflow-y-auto"
        aria-label={t("apps.privateResponses")}
      >
        {surfaces
          .filter(
            (surface) =>
              surface.kind === "ephemeral" && surface.channel_id === shell.selectedChannelId,
          )
          .map((surface) => (
            <section key={surface.id} className="rounded-xl border bg-background p-3 shadow-lg">
              <div className="flex justify-between gap-2">
                <p className="text-xs font-medium">
                  {surface.name} · {t("apps.onlyYou")}
                </p>
                <Button variant="ghost" size="sm" onClick={() => void dismiss(surface.id)}>
                  {t("apps.close")}
                </Button>
              </div>
              <AppBlocks
                appId={surface.app_id}
                surfaceId={surface.id}
                channelId={surface.channel_id}
                blocks={surface.document.blocks}
              />
            </section>
          ))}
      </aside>
    </>
  );
}
