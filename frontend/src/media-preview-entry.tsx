import { createRoot } from "react-dom/client";
import { MediaPreview } from "./components/ui/media-preview";
import { Button } from "./components/ui/button";
import { useTranslation } from "react-i18next";
import { LocaleProvider } from "./components/i18n/LocaleProvider";
import "./styles.css";

declare global {
  interface Window {
    NativePreview?: { save(name: string, type: string, base64: string): void; download(): void };
    openMediaPreview?: (name: string, contentType: string, locale: string, dark?: boolean) => void;
  }
}
const root = createRoot(document.getElementById("root")!);
window.openMediaPreview = (name, contentType, locale, dark = false) => {
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.lang = locale;
  localStorage.setItem("zoo-locale", locale);
  root.render(
    <LocaleProvider>
      <MediaPreview
        src="/source"
        name={name}
        contentType={contentType}
        footer={<NativeDownload name={name} />}
        embedded
      />
    </LocaleProvider>,
  );
};
// Only trusted bundled UI runs here. Email/document HTML is rendered as inert text.
document.addEventListener("click", (event) => {
  const anchor = (event.target as Element).closest("a");
  if (!anchor || !window.NativePreview) return;
  event.preventDefault();
  if (!anchor.href.startsWith("blob:")) return; // Original download remains in the native panel.
  void fetch(anchor.href)
    .then((response) => response.blob())
    .then(async (blob) => {
      if (blob.size > 16 * 1024 * 1024) return;
      const bytes = new Uint8Array(await blob.arrayBuffer());
      let raw = "";
      for (let i = 0; i < bytes.length; i += 8192)
        raw += String.fromCharCode(...bytes.subarray(i, i + 8192));
      window.NativePreview?.save(anchor.download || "attachment", blob.type, btoa(raw));
    });
});

function NativeDownload({ name }: { name: string }) {
  const { t } = useTranslation();
  return (
    <Button
      variant="ghost"
      size="sm"
      aria-label={t("attachments.download", { name })}
      onClick={() => window.NativePreview?.download()}
    >
      {t("attachments.downloadShort")}
    </Button>
  );
}
