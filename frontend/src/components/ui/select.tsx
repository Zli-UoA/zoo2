import type { ComponentProps } from "react";
import { cn } from "cn";
import { noAutofill } from "./autofill";
import { inputClassName } from "./input-styles";

export function Select({ className, ...props }: ComponentProps<"select">) {
  return (
    <select
      {...noAutofill}
      className={cn(
        inputClassName(),
        "w-40 max-w-full disabled:cursor-not-allowed disabled:opacity-60",
        className,
      )}
      {...props}
    />
  );
}
