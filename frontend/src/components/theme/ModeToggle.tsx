import { useTranslation } from "react-i18next";
import { MonitorIcon, MoonIcon, SunIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

import { useTheme } from "./ThemeProvider";
import { isTheme, type Theme } from "./theme";

const OPTIONS = [
  { value: "light", label: "theme.light", icon: SunIcon },
  { value: "dark", label: "theme.dark", icon: MoonIcon },
  { value: "system", label: "theme.system", icon: MonitorIcon },
] as const satisfies readonly {
  value: Theme;
  label: string;
  icon: typeof SunIcon;
}[];

function TriggerIcon({ theme }: { theme: Theme }) {
  const option = OPTIONS.find((candidate) => candidate.value === theme);
  const Icon = option?.icon ?? MonitorIcon;
  return <Icon aria-hidden="true" />;
}

/** ライト/ダーク/システムを切り替えるメニュー。 */
export function ModeToggle() {
  const { t } = useTranslation();
  const { theme, setTheme } = useTheme();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant="ghost" size="icon" aria-label={t("theme.switch")} />}
      >
        <TriggerIcon theme={theme} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t("theme.title")}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuRadioGroup
            value={theme}
            onValueChange={(value) => {
              if (isTheme(value)) {
                setTheme(value);
              }
            }}
          >
            {OPTIONS.map((option) => {
              const Icon = option.icon;
              return (
                <DropdownMenuRadioItem key={option.value} value={option.value} closeOnClick>
                  <Icon aria-hidden="true" />
                  {t(option.label)}
                </DropdownMenuRadioItem>
              );
            })}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
