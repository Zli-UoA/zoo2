import { useTranslation } from "react-i18next";
import { MonitorIcon, MoonIcon, SunIcon } from "lucide-react";
import { ButtonGroup, ButtonGroupItem } from "../ui/button-group";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../ui/tooltip";
import { useTheme } from "./ThemeProvider";
import { isTheme, type Theme } from "./theme";

const OPTIONS = [
  { value: "light", label: "theme.light", icon: SunIcon },
  { value: "dark", label: "theme.dark", icon: MoonIcon },
  { value: "system", label: "theme.system", icon: MonitorIcon },
] as const satisfies readonly { value: Theme; label: string; icon: typeof SunIcon }[];

/** Icon-only, single-choice theme selection with arrow-key navigation. */
export function ModeToggle() {
  const { t } = useTranslation();
  const { theme, setTheme } = useTheme();
  return (
    <TooltipProvider>
      <ButtonGroup
        aria-label={t("theme.title")}
        value={[theme]}
        onValueChange={([value]) => {
          if (isTheme(value)) setTheme(value);
        }}
      >
        {OPTIONS.map(({ value, label, icon: Icon }) => (
          <Tooltip key={value}>
            <TooltipTrigger render={<ButtonGroupItem value={value} aria-label={t(label)} />}>
              <Icon aria-hidden="true" />
            </TooltipTrigger>
            <TooltipContent>{t(label)}</TooltipContent>
          </Tooltip>
        ))}
      </ButtonGroup>
    </TooltipProvider>
  );
}
