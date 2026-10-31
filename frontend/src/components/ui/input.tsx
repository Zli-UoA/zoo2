import type { ComponentProps } from "react";
import { cn } from "cn";
import { noAutofill } from "./autofill";
import { inputClassName } from "./input-styles";

export function Input({ className, type, ...props }: ComponentProps<"input">) {
  const textInput = !type || !["checkbox", "radio", "file", "hidden"].includes(type);
  return (
    <input
      data-slot="input"
      type={type}
      className={cn(textInput && inputClassName(), className)}
      {...props}
      {...noAutofill}
    />
  );
}
