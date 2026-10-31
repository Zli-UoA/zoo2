import { LanguagesIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { LANGUAGE_NAMES, LOCALES, isLocale } from "../../lib/i18n/locale";
import { Select } from "../ui/select";
import { Button } from "../ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { useLocale } from "./LocaleProvider";

export function LanguageToggle({ presentation = "icon" }: { presentation?: "icon" | "select" }) {
  const { t } = useTranslation();
  const { locale, setLocale } = useLocale();
  if (presentation === "select") {
    return (
      <Select
        aria-label={t("language.title")}
        value={locale}
        onChange={(event) => {
          if (isLocale(event.target.value)) setLocale(event.target.value);
        }}
      >
        {LOCALES.map((value) => (
          <option key={value} value={value} lang={value}>
            {LANGUAGE_NAMES[value]}
          </option>
        ))}
      </Select>
    );
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant="ghost" size="icon" aria-label={t("language.switch")} />}
      >
        <LanguagesIcon aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t("language.title")}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuRadioGroup
            value={locale}
            onValueChange={(value) => {
              if (isLocale(value)) setLocale(value);
            }}
          >
            {LOCALES.map((value) => (
              <DropdownMenuRadioItem key={value} value={value} closeOnClick>
                <span lang={value}>{LANGUAGE_NAMES[value]}</span>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
