import { useTranslation } from "react-i18next";
import type { TypingUser } from "../../../hooks/useTyping";

export function TypingIndicator({ users }: { users: TypingUser[] }) {
  const { t } = useTranslation();
  return (
    <div
      role="status"
      aria-live="polite"
      aria-atomic="true"
      className="min-h-6 px-4 text-xs text-muted-foreground"
    >
      {users.length > 0
        ? t("messages.typing", {
            names: users.map((user) => user.display_name).join(t("messages.typingSeparator")),
          })
        : null}
    </div>
  );
}
