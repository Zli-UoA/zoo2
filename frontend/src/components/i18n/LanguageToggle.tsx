import { LanguagesIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { LANGUAGE_NAMES, LOCALES, isLocale } from "../../lib/i18n/locale";
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

export function LanguageToggle() {
  const { t } = useTranslation();
  const { locale, setLocale } = useLocale();
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
