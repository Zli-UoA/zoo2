import { useId } from "react";
import { useTranslation } from "react-i18next";
import { SEARCH_ENGINES, isSearchEngine } from "../../lib/searchEngine";
import { Button } from "../ui/button";
import { Select } from "../ui/select";
import { useSearchPreferences } from "./SearchPreferences";

export function SearchEngineSelect() {
  const { t } = useTranslation();
  const preferences = useSearchPreferences();
  const errorId = useId();
  const error = preferences?.error;
  return (
    <div className="flex min-w-0 flex-col items-end gap-2">
      <Select
        aria-label={t("search.engine")}
        value={preferences?.engine ?? "google"}
        disabled={!preferences || preferences.loading || preferences.saving || error === "load"}
        aria-busy={preferences?.loading || preferences?.saving}
        aria-describedby={error ? errorId : undefined}
        onChange={(event) => {
          if (isSearchEngine(event.target.value)) void preferences?.save(event.target.value);
        }}
      >
        {Object.entries(SEARCH_ENGINES).map(([value, engine]) => (
          <option key={value} value={value}>
            {engine.label}
          </option>
        ))}
      </Select>
      {error ? (
        <div id={errorId} role="alert" className="max-w-64 text-sm text-destructive">
          {t(error === "load" ? "search.loadError" : "search.saveError")}
          {error === "load" ? (
            <Button variant="link" onClick={preferences?.retry}>
              {t("search.retry")}
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
