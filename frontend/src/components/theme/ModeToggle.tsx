import { useTranslation } from "react-i18next";
import { MonitorIcon, MoonIcon, SunIcon } from "lucide-react";
import { ButtonGroup, ButtonGroupItem } from "../ui/button-group";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../ui/tooltip";
import { useTheme } from "./ThemeProvider";
import { isTheme, type Theme } from "./theme";

// 辞書のキーは `theme.` + 接尾辞。x18n が前方一致で追えるようにする。
const OPTIONS = [
  { value: "light", label: "light", icon: SunIcon },
  { value: "dark", label: "dark", icon: MoonIcon },
  { value: "system", label: "system", icon: MonitorIcon },
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
            <TooltipTrigger
              render={<ButtonGroupItem value={value} aria-label={t(`theme.${label}`)} />}
            >
              <Icon aria-hidden="true" />
            </TooltipTrigger>
            <TooltipContent>{t(`theme.${label}`)}</TooltipContent>
          </Tooltip>
        ))}
      </ButtonGroup>
    </TooltipProvider>
  );
}
