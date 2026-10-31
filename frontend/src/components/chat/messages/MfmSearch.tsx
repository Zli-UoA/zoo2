import { SearchIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SEARCH_ENGINES, searchUrl } from "../../../lib/searchEngine";
import { useSearchEngine } from "../../preferences/SearchPreferences";
import { Input } from "../../ui/input";
import { Button } from "../../ui/button";

export function MfmSearch({ query }: { query: string }) {
  const { t } = useTranslation();
  const selectedEngine = useSearchEngine();
  const engine = SEARCH_ENGINES[selectedEngine];
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        // External form submissions are blocked by the production form-action CSP.
        window.open(searchUrl(selectedEngine, query), "_blank", "noopener,noreferrer");
      }}
      action={engine.action}
      method="get"
      target="_blank"
      rel="noopener noreferrer"
      autoComplete="off"
      className="mfm-search my-1 inline-flex max-w-full items-center gap-1.5 align-middle whitespace-normal"
    >
      <Input
        readOnly
        name={engine.parameter}
        value={query}
        aria-label={t("search.query")}
        size={Math.max(1, Math.min(80, query.length))}
        className="w-auto min-w-0 flex-[0_1_auto] [field-sizing:content]"
      />
      <Button
        type="submit"
        variant="outline"
        title={engine.label}
        aria-label={t("search.withEngine", { engine: engine.label })}
      >
        <SearchIcon aria-hidden="true" />
        {t("search.action")}
      </Button>
    </form>
  );
}
