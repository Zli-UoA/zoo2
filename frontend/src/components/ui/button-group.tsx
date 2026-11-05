import { ToggleGroup } from "@base-ui/react/toggle-group";
import { Toggle } from "@base-ui/react/toggle";
import { cn } from "cn";
import { Button } from "./button";

export function ButtonGroup({
  className,
  ...props
}: Omit<ToggleGroup.Props<string>, "className"> & { className?: string }) {
  return (
    <ToggleGroup
      className={cn(
        "inline-flex shrink-0 items-center gap-0.5 rounded-lg border bg-muted/40 p-0.5",
        className,
      )}
      {...props}
    />
  );
}

export function ButtonGroupItem({
  className,
  ...props
}: Omit<Toggle.Props<string>, "className"> & { className?: string }) {
  return (
    <Toggle
      render={<Button variant="ghost" size="icon" />}
      className={cn(
        "size-10 rounded-md data-[pressed]:bg-primary data-[pressed]:text-primary-foreground data-[pressed]:shadow-sm",
        className,
      )}
      {...props}
    />
  );
}
