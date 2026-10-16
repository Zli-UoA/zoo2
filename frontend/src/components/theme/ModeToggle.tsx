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
  { value: "light", label: "ライト", icon: SunIcon },
  { value: "dark", label: "ダーク", icon: MoonIcon },
  { value: "system", label: "システム", icon: MonitorIcon },
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
  const { theme, setTheme } = useTheme();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant="ghost" size="icon" aria-label="テーマを切り替える" />}
      >
        <TriggerIcon theme={theme} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuGroup>
          <DropdownMenuLabel>テーマ</DropdownMenuLabel>
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
                  {option.label}
                </DropdownMenuRadioItem>
              );
            })}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
