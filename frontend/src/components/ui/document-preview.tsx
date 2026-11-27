import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { DocumentPreviewData, DocumentResponse, PreviewFile } from "../../lib/documents/types";
import { PreviewDownloadButton } from "../desktop/PreviewDownloadButton";
import { Button } from "./button";
import { Spinner } from "./spinner";

/** Parsing/fetching lives in a disposable worker, only while the user opens this viewer. */
export function DocumentPreview({
  src,
  name,
  contentType,
  renderFile,
}: {
  src: string;
  name: string;
  contentType: string;
  renderFile?: (file: PreviewFile, url: string) => ReactNode;
}) {
  const { t } = useTranslation();
  const [result, setResult] = useState<DocumentResponse | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setResult(null);
    const worker = new Worker(new URL("../../lib/documents/preview.worker.ts", import.meta.url), {
      type: "module",
    });
    const timeout = setTimeout(() => {
      worker.terminate();
      setResult({ error: "limit" });
    }, 15_000);
    worker.onmessage = (event: MessageEvent<DocumentResponse>) => {
      clearTimeout(timeout);
      setResult(event.data);
      worker.terminate();
    };
    worker.onerror = () => {
      clearTimeout(timeout);
      setResult({ error: "invalid" });
      worker.terminate();
    };
    worker.postMessage({ src, name, contentType });
    return () => {
      clearTimeout(timeout);
      worker.terminate();
    };
  }, [src, name, contentType, attempt]);
  if (!result) return <Spinner label={t("attachments.loading")} className="min-h-32" />;
  if ("error" in result)
    return (
      <div role="alert" className="space-y-3 p-3">
        <p>{t(result.error === "limit" ? "attachments.previewLimit" : "attachments.loadError")}</p>
        <Button variant="outline" onClick={() => setAttempt((n) => n + 1)}>
          {t("attachments.retry")}
        </Button>
      </div>
    );
  return <DocumentBody data={result.data} renderFile={renderFile} />;
}
function DocumentBody({
  data,
  renderFile,
}: {
  data: DocumentPreviewData;
  renderFile?: (file: PreviewFile, url: string) => ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div className="max-h-[70dvh] min-w-0 space-y-4 overflow-auto p-2 [overflow-wrap:anywhere]">
      {data.headers ? (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-2 text-sm">
          {data.headers
            .filter((h) => h.value)
            .map((header) => (
              <div key={header.label} className="contents">
                <dt className="font-semibold">{t(`attachments.mail.${header.label}`)}</dt>
                <dd className="whitespace-pre-wrap">{header.value}</dd>
              </div>
            ))}
        </dl>
      ) : (
        <p className="text-xs text-muted-foreground">{t("attachments.documentSummary")}</p>
      )}
      {data.sections.map((section, index) => (
        <section key={index} aria-label={section.name}>
          <h3 className="mb-2 break-all text-xs font-semibold">{section.name}</h3>
          <pre className="whitespace-pre-wrap break-words font-mono text-sm">
            {section.text || t("attachments.emptyPreview")}
          </pre>
        </section>
      ))}
      {data.files.length ? (
        <section aria-label={t("attachments.containedFiles")}>
          <h3 className="mb-2 font-semibold">{t("attachments.containedFiles")}</h3>
          <ul className="space-y-2">
            {data.files.map((file, index) => (
              <li key={index}>
                <ContainedFile file={file} renderFile={renderFile} />
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}
function ContainedFile({
  file,
  renderFile,
}: {
  file: PreviewFile;
  renderFile?: (file: PreviewFile, url: string) => ReactNode;
}) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    // URLs exist only in the mounted viewer and are revoked on close/account changes.
    const value = URL.createObjectURL(new Blob([file.bytes], { type: file.type }));
    setUrl(value);
    return () => URL.revokeObjectURL(value);
  }, [file]);
  return (
    <div className="rounded border p-2">
      {url && renderFile ? (
        renderFile(file, url)
      ) : (
        <span className="break-all text-sm">{file.name}</span>
      )}
      {url && !renderFile ? (
        <PreviewDownloadButton name={file.name} src={url} className="ml-2 text-xs text-link" />
      ) : null}
    </div>
  );
}
