import { useTranslation } from "react-i18next";
import { selectedFrontend } from "../../lib/experimentalFrontend";
import { PREVIEW_USERS, PREVIEW_USER_KEY, previewUserId } from "@/lib/preview";

export function PreviewUserSwitcher() {
  const { t } = useTranslation();
  if (import.meta.env.VITE_PREVIEW_AUTH !== "true" || import.meta.env.VITE_OIDC_ISSUER) {
    return null;
  }
  if (typeof window !== "undefined" && selectedFrontend() === "compose") return null;
  return (
    <aside className="fixed right-3 bottom-3 z-50 rounded-md border bg-background p-2 text-sm shadow-md">
      <label className="flex items-center gap-2">
        {t("auth.previewUser")}
        <select
          data-testid="preview-user"
          aria-label={t("auth.previewUser")}
          className="rounded border bg-background p-1"
          defaultValue={previewUserId()}
          onChange={(event) => {
            try {
              window.sessionStorage.setItem(PREVIEW_USER_KEY, event.target.value);
            } catch {
              // Storage is blocked; keep showing the user that requests still use.
              event.target.value = previewUserId();
              return;
            }
            window.location.reload();
          }}
        >
          {PREVIEW_USERS.map((user) => (
            <option key={user.id} value={user.id}>
              {t(user.labelKey, { name: user.name })}
            </option>
          ))}
        </select>
      </label>
    </aside>
  );
}
