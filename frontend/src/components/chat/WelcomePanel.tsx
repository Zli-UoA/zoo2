import { useTranslation } from "react-i18next";

/** チャンネル未選択のときのメイン面。 */
export function WelcomePanel() {
  const { t } = useTranslation();
  return (
    <div className="flex h-full flex-1 items-center justify-center p-6">
      <div className="flex max-w-md flex-col items-center gap-3 text-center">
        <img src="/icons/zoo.svg" alt="" width={48} height={48} className="size-12 rounded-2xl" />
        <h1 className="font-heading text-xl font-semibold tracking-tight">{t("welcome.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("welcome.description")}</p>
      </div>
    </div>
  );
}
