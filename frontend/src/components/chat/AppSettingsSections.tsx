import { Input } from "../ui/input";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/button";
import { SettingsSection } from "../ui/control-panel";
import { inputClassName } from "../ui/input-styles";

export function AppPermissions({ scopes = [] }: { scopes?: string[] }) {
  const { t } = useTranslation();
  return (
    <SettingsSection title={t("apps.permissions")}>
      <ul className="flex flex-wrap gap-1.5 text-xs text-muted-foreground">
        {scopes.map((scope) => (
          <li key={scope} className="rounded-md border bg-muted/40 px-2 py-1 break-all">
            {scope}
          </li>
        ))}
      </ul>
    </SettingsSection>
  );
}

export function AppCredentials({ value, onError }: { value: string; onError: () => void }) {
  const { t } = useTranslation();
  const inputId = useId();
  return (
    <SettingsSection>
      <label htmlFor={inputId} className="block text-xs text-muted-foreground">
        {t("apps.credential")}
      </label>
      <div className="flex gap-2">
        <Input
          id={inputId}
          className={`${inputClassName()} min-w-0 flex-1 font-mono`}
          type="password"
          readOnly
          value={value}
          autoComplete="off"
        />
        <Button
          variant="outline"
          onClick={() => void navigator.clipboard.writeText(value).catch(onError)}
        >
          {t("apps.copy")}
        </Button>
      </div>
    </SettingsSection>
  );
}
