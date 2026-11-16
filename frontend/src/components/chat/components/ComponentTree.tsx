import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { useLiveQuery } from "@tanstack/react-db";
import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { refreshAppSurfaces } from "../../../lib/appSurfaceEvents";
import { createConfiguredApiClient } from "../../../lib/api/client";
import { apiPath } from "../../../lib/api/path";
import { useCollections } from "../../../lib/electric/collections-context";
import { Button, buttonVariants } from "../../ui/button";
import { ConfirmActionDialog } from "../../ui/confirm-action-dialog";
import { ExternalLink } from "../../ui/external-link";
import { Input } from "../../ui/input";
import { Select } from "../../ui/select";
import { SensitiveMedia } from "../../ui/sensitive-media";
import { Textarea } from "../../ui/textarea";
import { MfmContent } from "../messages/MfmContent";
import { proxyImageUrl } from "../messages/linkPreviews";
import { subscribePreviewEndpoints } from "../messages/linkPreviewEndpoints";
import {
  T,
  accentColor,
  isInput,
  multiValue,
  safeUrl,
  type Component,
  type ComponentDocument,
  type Confirm,
  type SelectOption,
  type TableCell,
  type Task,
} from "./parse";

type Value = string | string[] | boolean;

/** 操作に必要な情報。`appId` がなければ、操作部品は押せない状態で表示する。 */
export interface ComponentTarget {
  appId?: string | null;
  messageId?: string;
  surfaceId?: string;
  channelId?: string | null;
  /** モーダル・ホーム。入力をまとめて送信ボタンで送る */
  submit?: boolean;
  onDone?: () => void;
  /** `attachment://名前` を、このメッセージのアプリのファイルに結び付ける */
  attachments?: { id: string; name: string }[];
}

interface Ctx extends ComponentTarget {
  busy: boolean;
  values: Record<string, Value>;
  set: (id: string, value: Value) => void;
  invoke: (kind: string, actionId?: string, value?: Value) => Promise<unknown>;
  media: string;
  download: (id: string) => void;
}
const ComponentContext = createContext<Ctx | null>(null);
const useCtx = () => useContext(ComponentContext)!;

function initialValues(doc: ComponentDocument): Record<string, Value> {
  const out: Record<string, Value> = {};
  const walk = (c?: Component) => {
    if (!c) return;
    if (c.custom_id && isInput(c)) {
      if (c.type === T.TextInput) out[c.custom_id] = c.value ?? "";
      else if (c.type === T.Checkbox) out[c.custom_id] = !!(c as { default?: boolean }).default;
      else if ([T.DatePicker, T.TimePicker, T.DateTimePicker].includes(c.type as never))
        out[c.custom_id] = c.initial_value ?? "";
      else {
        const chosen = (c.options ?? c.initial_options ?? [])
          .filter((o) => o.default || c.initial_options)
          .map((o) => o.value);
        const defaults = (c.default_values ?? []).map((d) => d.id);
        const all = [...chosen, ...defaults];
        out[c.custom_id] = multiValue(c) ? all : (all[0] ?? "");
      }
    }
    c.components?.forEach(walk);
    walk(c.accessory);
    walk(c.component);
    c.actions?.forEach(walk);
    c.cards?.forEach(walk);
    if (c.type === T.Grid)
      (c.rows as { component: Component }[][] | undefined)?.forEach((row) =>
        row.forEach((item) => walk(item.component)),
      );
  };
  doc.components.forEach(walk);
  return out;
}

/** メッセージ部品の木を描画する。本文を隠すかどうかは呼び出し側が `hidesContent` で決める。 */
export function ComponentTree({
  document: doc,
  ...target
}: { document: ComponentDocument } & ComponentTarget) {
  const { t } = useTranslation();
  const collections = useCollections();
  const config = collections?.config;
  const client = useMemo(() => (config ? createConfiguredApiClient(config) : null), [config]);
  const [values, setValues] = useState(() => initialValues(doc));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [media, setMedia] = useState("");
  useEffect(
    () => (config ? subscribePreviewEndpoints(config, (s) => setMedia(s?.media ?? "")) : undefined),
    [config],
  );
  async function invoke(kind: string, actionId?: string, value?: Value) {
    if (busy || !client || !target.appId) return;
    setBusy(true);
    setError("");
    try {
      const result = await client.request(apiPath`/api/apps/${target.appId}/invoke`, {
        method: "POST",
        body: {
          kind,
          surface_id: target.surfaceId,
          message_id: target.messageId,
          channel_id: target.channelId,
          action_id: actionId,
          values: { ...values, ...(actionId && value !== undefined ? { [actionId]: value } : {}) },
        },
      });
      refreshAppSurfaces();
      if (kind === "submit") target.onDone?.();
      return result;
    } catch {
      setError(t("apps.actionError"));
    } finally {
      setBusy(false);
    }
  }
  async function download(id: string) {
    try {
      const result = await client!.get<{ url: string }>(apiPath`/api/app-files/${id}/download`);
      if (result.url.startsWith("https://"))
        window.open(result.url, "_blank", "noopener,noreferrer");
    } catch {
      setError(t("apps.actionError"));
    }
  }
  const ctx: Ctx = {
    ...target,
    busy,
    values,
    set: (id, value) => setValues((v) => ({ ...v, [id]: value })),
    invoke,
    media,
    download: (id) => void download(id),
  };
  const body = (
    <div className="my-2 flex min-w-0 flex-col gap-2 text-sm">
      {doc.components.map((c, i) => (
        <Node key={i} c={c} />
      ))}
    </div>
  );
  return (
    <ComponentContext.Provider value={ctx}>
      {target.submit ? (
        <form
          autoComplete="off"
          onSubmit={(e) => {
            e.preventDefault();
            void invoke("submit");
          }}
        >
          {body}
          <Button type="submit" disabled={busy}>
            {t("apps.submit")}
          </Button>
        </form>
      ) : (
        body
      )}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </ComponentContext.Provider>
  );
}

function Text({ content, small = false }: { content: string; small?: boolean }) {
  return (
    <div
      className={`mfm-body min-w-0 whitespace-pre-wrap break-words leading-relaxed ${small ? "text-xs text-muted-foreground" : ""}`}
    >
      <MfmContent content={content} />
    </div>
  );
}

function Image({
  url,
  alt,
  spoiler,
  className,
}: {
  url: string;
  alt?: string;
  spoiler?: boolean;
  className: string;
}) {
  const { media } = useCtx();
  const safe = safeUrl(url);
  const src = proxyImageUrl(media, safe);
  // 画像のプロキシが使えないときは、置き場所へ直接取りに行かず、説明とリンクだけを出す
  if (!src)
    return safe ? (
      <ExternalLink href={safe} className="text-xs">
        {alt || safe}
      </ExternalLink>
    ) : null;
  return (
    <SensitiveMedia sensitive={spoiler}>
      <img
        src={src}
        alt={alt ?? ""}
        loading="lazy"
        className={`rounded-md bg-muted object-cover ${className}`}
      />
    </SensitiveMedia>
  );
}

function Node({ c }: { c: Component }): ReactNode {
  const { t } = useTranslation();
  switch (c.type) {
    case T.TextDisplay:
      return c.content ? <Text content={c.content} /> : null;
    case T.Container:
      return <Container c={c} />;
    case T.Section:
      return (
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            {c.components?.map((x, i) => (
              <Node key={i} c={x} />
            ))}
          </div>
          {c.accessory ? (
            <div className="shrink-0">
              {c.accessory.type === T.Thumbnail ? (
                <Image
                  url={c.accessory.media?.url ?? ""}
                  alt={c.accessory.description}
                  spoiler={c.accessory.spoiler}
                  className="size-16"
                />
              ) : (
                <Interactive c={c.accessory} />
              )}
            </div>
          ) : null}
        </div>
      );
    case T.MediaGallery:
      return (
        <div className="grid max-w-md grid-cols-2 gap-1 [&:has(>:only-child)]:grid-cols-1">
          {c.items?.slice(0, 10).map((item, i) => (
            <Image
              key={i}
              url={item.media?.url}
              alt={item.description}
              spoiler={item.spoiler}
              className="aspect-video w-full"
            />
          ))}
        </div>
      );
    case T.File:
      return <FileLink c={c} />;
    case T.Separator:
      return c.divider === false ? (
        <div className={c.spacing === 2 ? "h-4" : "h-1"} aria-hidden />
      ) : (
        <hr className={`border-border ${c.spacing === 2 ? "my-3" : "my-1"}`} />
      );
    case T.ActionRow:
      return (
        <div className="flex flex-wrap items-center gap-2">
          {c.components?.map((x, i) => (
            <Interactive key={i} c={x} />
          ))}
        </div>
      );
    case T.Label:
      // 1 つのチェックボックスは、見出しと横に並べる
      if (c.component?.type === T.Checkbox)
        return (
          <label className="flex items-center gap-2">
            <Interactive c={c.component} labelled />
            <span className="flex flex-col">
              <span className="font-medium">{c.label}</span>
              {c.description ? (
                <span className="text-xs text-muted-foreground">{c.description}</span>
              ) : null}
            </span>
          </label>
        );
      return (
        <label className="flex flex-col gap-1">
          <span className="font-medium">{c.label}</span>
          {c.description ? (
            <span className="text-xs text-muted-foreground">{c.description}</span>
          ) : null}
          {c.component ? <Interactive c={c.component} labelled /> : null}
        </label>
      );
    case T.Video:
      return <Video c={c} />;
    case T.Table:
      return <Table c={c} />;
    case T.Alert:
      return (
        <div
          role="note"
          className={`rounded-md border px-3 py-2 ${
            c.level === "error"
              ? "border-destructive/50 bg-destructive/10"
              : c.level === "warning"
                ? "border-amber-500/50 bg-amber-500/10"
                : c.level === "success"
                  ? "border-emerald-500/50 bg-emerald-500/10"
                  : "bg-muted/50"
          }`}
        >
          <Text content={c.content ?? ""} />
        </div>
      );
    case T.Card:
      return <Card c={c} />;
    case T.Carousel:
      return (
        <div className="flex max-w-full snap-x gap-2 overflow-x-auto pb-1">
          {c.cards?.map((x, i) => (
            <div key={i} className="w-60 shrink-0 snap-start">
              <Card c={x} />
            </div>
          ))}
        </div>
      );
    case T.Chart:
      return <Chart c={c} />;
    case T.Grid:
      return (
        <div className="@container flex flex-col gap-2">
          {c.label ? <div className="font-medium">{c.label}</div> : null}
          {(
            c.rows as { width?: number; align?: string; component: Component }[][] | undefined
          )?.map((row, r) => (
            <div
              key={r}
              className={`flex flex-col gap-2 @md:flex-row ${c.grid_lines === "rows" ? "border-b pb-2" : ""}`}
            >
              {row.map((item, i) => (
                <div
                  key={i}
                  style={{ flexGrow: item.width ?? 2 }}
                  className={`min-w-0 basis-0 ${item.align === "center" ? "text-center" : item.align === "end" ? "text-right" : ""}`}
                >
                  <Node c={item.component} />
                </div>
              ))}
            </div>
          ))}
        </div>
      );
    case T.Plan:
      return (
        <div className="flex flex-col gap-1 rounded-md border p-2">
          <div className="font-medium">{c.title}</div>
          {c.tasks?.map((task) => (
            <TaskRow key={task.task_id} task={task} />
          ))}
        </div>
      );
    case T.TaskCard:
      return <TaskRow task={c as unknown as Task} />;
    case T.Thumbnail:
      return (
        <Image
          url={c.media?.url ?? ""}
          alt={c.description}
          spoiler={c.spoiler}
          className="size-16"
        />
      );
    case T.Unsupported:
      return c.text ? (
        <div
          className="rounded-md border border-dashed px-2 py-1"
          title={t("apps.components.unsupported")}
        >
          <Text content={c.text} small />
        </div>
      ) : null;
    default:
      return isInput(c) || c.type === T.Button || c.type === T.Overflow ? (
        <Interactive c={c} />
      ) : null;
  }
}

function Container({ c }: { c: Component }) {
  const [open, setOpen] = useState(!c.collapsed);
  const color = accentColor(c.accent_color);
  const children = (
    <div className="flex min-w-0 flex-col gap-2">
      {c.components?.map((x, i) => (
        <Node key={i} c={x} />
      ))}
    </div>
  );
  return (
    <div
      className="min-w-0 max-w-xl rounded-lg border border-l-4 bg-card p-3"
      style={{ borderLeftColor: color ?? "var(--color-primary)" }}
    >
      {c.title ? (
        c.collapsible ? (
          <button
            type="button"
            className="mb-1 flex items-center gap-1 text-left font-medium"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            {open ? (
              <ChevronDownIcon className="size-4" />
            ) : (
              <ChevronRightIcon className="size-4" />
            )}
            <MfmContent content={c.title} />
          </button>
        ) : (
          <div className="mb-1 font-medium">
            <MfmContent content={c.title} />
          </div>
        )
      ) : null}
      {c.subtitle ? <Text content={c.subtitle} small /> : null}
      {open ? <SensitiveMedia sensitive={c.spoiler}>{children}</SensitiveMedia> : null}
    </div>
  );
}

function FileLink({ c }: { c: Component }) {
  const { attachments, download } = useCtx();
  const url = c.file?.url ?? "";
  const name = c.name ?? url.split("/").pop() ?? "file";
  if (url.startsWith("attachment://")) {
    const file = attachments?.find((a) => a.name === url.slice("attachment://".length));
    return file ? (
      <Button
        type="button"
        variant="link"
        className="h-auto justify-start p-0"
        onClick={() => download(file.id)}
      >
        {file.name}
      </Button>
    ) : null;
  }
  const safe = safeUrl(url);
  return safe ? <ExternalLink href={safe}>{name}</ExternalLink> : <span>{name}</span>;
}

function Video({ c }: { c: Component }) {
  const { media } = useCtx();
  const href = safeUrl(c.title_url) ?? safeUrl(c.video_url);
  const thumbnail = proxyImageUrl(media, safeUrl(c.thumbnail_url));
  return (
    <div className="flex max-w-sm flex-col gap-1 rounded-lg border p-2">
      {thumbnail ? (
        <a
          href={href ?? undefined}
          target="_blank"
          rel="noopener noreferrer"
          className="relative block"
        >
          <Image url={c.thumbnail_url ?? ""} alt={c.alt_text} className="aspect-video w-full" />
          <span
            aria-hidden
            className="absolute inset-0 m-auto flex size-10 items-center justify-center rounded-full bg-black/60 text-white"
          >
            ▶
          </span>
        </a>
      ) : null}
      {href ? <ExternalLink href={href}>{c.title}</ExternalLink> : <span>{c.title}</span>}
      {c.description ? (
        <span className="text-xs text-muted-foreground">{c.description}</span>
      ) : null}
      {c.author_name || c.provider_name ? (
        <span className="text-xs text-muted-foreground">
          {[c.author_name, c.provider_name].filter(Boolean).join(" · ")}
        </span>
      ) : null}
    </div>
  );
}

function Table({ c }: { c: Component }) {
  const { t } = useTranslation();
  const rows = (c.rows as TableCell[][] | undefined) ?? [];
  const [sort, setSort] = useState<{ col: number; asc: boolean } | null>(null);
  const [page, setPage] = useState(0);
  const head = c.header ? rows[0] : null;
  let body = c.header ? rows.slice(1) : rows;
  if (sort)
    body = [...body].sort((a, b) => {
      const x = a[sort.col],
        y = b[sort.col];
      const r =
        x?.number !== undefined && y?.number !== undefined
          ? x.number - y.number
          : (x?.text ?? "").localeCompare(y?.text ?? "");
      return sort.asc ? r : -r;
    });
  const size = c.page_size ?? body.length;
  const pages = Math.max(1, Math.ceil(body.length / Math.max(1, size)));
  const shown = body.slice(page * size, page * size + size);
  const align = (i: number) => {
    const a = c.column_settings?.[i]?.align;
    return a === "center" ? "text-center" : a === "right" ? "text-right" : "text-left";
  };
  const cell = (x: TableCell | undefined) =>
    x?.button ? <Interactive c={x.button} /> : <MfmContent content={x?.text ?? ""} />;
  return (
    <div className="max-w-full overflow-x-auto">
      <table className="min-w-max border-collapse text-sm">
        {c.caption ? (
          <caption className="text-left text-xs text-muted-foreground">{c.caption}</caption>
        ) : null}
        {head ? (
          <thead>
            <tr>
              {head.map((h, i) => (
                <th key={i} className={`border-b px-2 py-1 font-medium ${align(i)}`}>
                  <button
                    type="button"
                    onClick={() => setSort({ col: i, asc: sort?.col === i ? !sort.asc : true })}
                  >
                    <MfmContent content={h.text} />
                  </button>
                </th>
              ))}
            </tr>
          </thead>
        ) : null}
        <tbody>
          {shown.map((r, i) => (
            <tr key={i} className="border-b last:border-0">
              {r.map((x, j) => (
                <td
                  key={j}
                  className={`px-2 py-1 align-middle ${align(j)} ${c.column_settings?.[j]?.is_wrapped ? "whitespace-normal" : "whitespace-nowrap"}`}
                >
                  {cell(x)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {pages > 1 ? (
        <div className="flex items-center gap-2 text-xs">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={page === 0}
            onClick={() => setPage(page - 1)}
          >
            ‹
          </Button>
          {t("apps.components.page", { page: page + 1, pages })}
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={page + 1 >= pages}
            onClick={() => setPage(page + 1)}
          >
            ›
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function Card({ c }: { c: Component }) {
  return (
    <div className="flex min-w-0 max-w-sm flex-col gap-1 overflow-hidden rounded-lg border bg-card">
      {c.hero_image ? (
        <Image url={c.hero_image.url} className="aspect-video w-full rounded-none" />
      ) : null}
      <div className="flex flex-col gap-1 p-2">
        <div className="flex items-center gap-2">
          {c.icon ? <Image url={c.icon.url} className="size-6" /> : null}
          {c.title ? (
            <div className="font-medium">
              <MfmContent content={c.title} />
            </div>
          ) : null}
        </div>
        {c.subtitle ? <Text content={c.subtitle} small /> : null}
        {c.body ? <Text content={c.body} /> : null}
        {c.subtext ? <Text content={c.subtext} small /> : null}
        {c.actions?.length ? (
          <div className="flex flex-wrap justify-end gap-2">
            {c.actions.map((a, i) => (
              <Interactive key={i} c={a} />
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
];

/** 依存を増やさない簡単なグラフ（棒・折れ線・面は SVG、円は割合の帯）。数値は表でも読める。 */
function Chart({ c }: { c: Component }) {
  const color = (i: number) => CHART_COLORS[i % CHART_COLORS.length] ?? "currentColor";
  if (c.chart_type === "pie") {
    const total = (c.segments ?? []).reduce((s, x) => s + Math.max(0, x.value), 0) || 1;
    return (
      <figure className="flex max-w-sm flex-col gap-1">
        <figcaption className="font-medium">{c.title}</figcaption>
        <div className="flex h-4 overflow-hidden rounded">
          {c.segments?.map((s, i) => (
            <span
              key={i}
              style={{ width: `${(Math.max(0, s.value) / total) * 100}%`, background: color(i) }}
              title={`${s.label}: ${s.value}`}
            />
          ))}
        </div>
        <ul className="flex flex-wrap gap-x-3 text-xs">
          {c.segments?.map((s, i) => (
            <li key={i} className="flex items-center gap-1">
              <span className="inline-block size-2 rounded-full" style={{ background: color(i) }} />
              {s.label}: {s.value}
            </li>
          ))}
        </ul>
      </figure>
    );
  }
  const categories = c.categories?.length
    ? c.categories
    : [...new Set((c.series ?? []).flatMap((s) => s.data.map((p) => p.label)))];
  const all = (c.series ?? []).flatMap((s) => s.data.map((p) => p.value));
  const max = Math.max(0, ...all),
    min = Math.min(0, ...all);
  const span = max - min || 1;
  const W = 320,
    H = 140,
    pad = 4;
  const x = (i: number) => pad + ((W - 2 * pad) * (i + 0.5)) / Math.max(1, categories.length);
  const y = (v: number) => H - pad - ((H - 2 * pad) * (v - min)) / span;
  return (
    <figure className="flex max-w-sm flex-col gap-1">
      <figcaption className="font-medium">{c.title}</figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} className="w-full" role="img" aria-label={c.title}>
        {(c.series ?? []).map((s, si) => {
          const pts = categories.map((cat, i) => {
            const v = s.data.find((p) => p.label === cat)?.value ?? 0;
            return [x(i), y(v)] as const;
          });
          if (c.chart_type === "bar") {
            const w =
              ((W - 2 * pad) / Math.max(1, categories.length) / Math.max(1, c.series!.length)) *
              0.8;
            return pts.map(([px, py], i) => (
              <rect
                key={`${si}-${i}`}
                x={px - (w * c.series!.length) / 2 + si * w}
                y={Math.min(py, y(0))}
                width={w}
                height={Math.abs(y(0) - py)}
                fill={color(si)}
              />
            ));
          }
          const d = pts.map(([px, py], i) => `${i ? "L" : "M"}${px},${py}`).join(" ");
          return c.chart_type === "area" ? (
            <path
              key={si}
              d={`${d} L${pts.at(-1)?.[0]},${y(min)} L${pts[0]?.[0]},${y(min)} Z`}
              fill={color(si)}
              opacity={0.4}
            />
          ) : (
            <path key={si} d={d} fill="none" stroke={color(si)} strokeWidth={2} />
          );
        })}
      </svg>
      <div className="flex justify-between text-xs text-muted-foreground">
        {categories.map((cat) => (
          <span key={cat}>{cat}</span>
        ))}
      </div>
      <ul className="flex flex-wrap gap-x-3 text-xs">
        {c.series?.map((s, i) => (
          <li key={i} className="flex items-center gap-1">
            <span className="inline-block size-2 rounded-full" style={{ background: color(i) }} />
            {s.name}
          </li>
        ))}
      </ul>
      {c.x_label || c.y_label ? (
        <span className="text-xs text-muted-foreground">
          {[c.y_label, c.x_label].filter(Boolean).join(" / ")}
        </span>
      ) : null}
    </figure>
  );
}

function TaskRow({ task }: { task: Task }) {
  const mark =
    task.status === "complete"
      ? "✓"
      : task.status === "error"
        ? "✕"
        : task.status === "in_progress"
          ? "…"
          : "○";
  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-center gap-2">
        <span aria-hidden className={task.status === "error" ? "text-destructive" : ""}>
          {mark}
        </span>
        <span>{task.title}</span>
      </div>
      {task.details ? <Text content={task.details} small /> : null}
      {task.output ? <Text content={task.output} small /> : null}
      {task.sources?.map((s) => {
        const href = safeUrl(s.url);
        return href ? (
          <ExternalLink key={s.url} href={href} className="text-xs">
            {s.text}
          </ExternalLink>
        ) : null;
      })}
    </div>
  );
}

/** 押す前の確認（confirm）があれば、確認ダイアログを挟む。 */
function useConfirm(confirm?: Confirm) {
  const [pending, setPending] = useState<(() => void) | null>(null);
  const dialog = confirm ? (
    <ConfirmActionDialog
      open={!!pending}
      onOpenChange={(open) => !open && setPending(null)}
      title={confirm.title}
      description={confirm.text}
      confirmLabel={confirm.confirm}
      cancelLabel={confirm.deny}
      confirmVariant={confirm.style === "danger" ? "destructive" : "default"}
      onConfirm={() => {
        pending?.();
        setPending(null);
      }}
    />
  ) : null;
  return {
    run: (action: () => void) => (confirm ? setPending(() => action) : action()),
    dialog,
  };
}

const BUTTON_VARIANT: Record<number, "default" | "secondary" | "outline" | "destructive"> = {
  1: "default",
  2: "outline",
  3: "secondary",
  4: "destructive",
};

/** ボタン・select・入力欄・ピッカー。`labelled` は Label の中（送信でまとめて送る）。 */
function Interactive(props: { c: Component; labelled?: boolean }): ReactNode {
  const confirm = useConfirm(props.c.confirm);
  return (
    <>
      <InteractiveControl {...props} confirm={confirm.run} />
    </>
  );
}

function InteractiveControl({
  c,
  labelled = false,
  confirm: runConfirmed,
}: {
  c: Component;
  labelled?: boolean;
  confirm: (action: () => void) => void;
}): ReactNode {
  const { t } = useTranslation();
  const ctx = useCtx();
  const confirm = { run: runConfirmed };
  const id = c.custom_id ?? "";
  const disabled = !!c.disabled || ctx.busy || !ctx.appId;
  // モーダル・ホームの入力欄は送信でまとめて送る。メッセージの select などは選んだときに送る
  const deferred = labelled || !!ctx.submit;
  const choose = (value: Value) => {
    ctx.set(id, value);
    if (!deferred) confirm.run(() => void ctx.invoke("action", id, value));
  };
  const value = ctx.values[id];
  switch (c.type) {
    case T.Button: {
      const label = [c.emoji?.name, c.label].filter(Boolean).join(" ");
      if (c.style === 5) {
        const href = safeUrl(c.url);
        return href ? (
          <a
            href={href}
            target="_blank"
            rel="noopener noreferrer"
            className={buttonVariants({ size: "sm", variant: "outline" })}
          >
            {label}
          </a>
        ) : null;
      }
      return (
        <>
          <Button
            type="button"
            size="sm"
            variant={BUTTON_VARIANT[c.style ?? 2] ?? "outline"}
            className={c.style === 3 ? "bg-emerald-600 text-white hover:bg-emerald-700" : undefined}
            disabled={disabled}
            onClick={() => confirm.run(() => void ctx.invoke("action", id, c.value ?? ""))}
          >
            {label}
          </Button>
        </>
      );
    }
    case T.StringSelect:
    case T.Overflow:
    case T.RadioGroup: {
      const many = c.type === T.StringSelect && (c.max_values ?? 1) > 1;
      if (many)
        return <CheckList c={c} disabled={disabled && !deferred} choose={choose} value={value} />;
      if (c.type === T.RadioGroup)
        return (
          <fieldset className="flex flex-col gap-1" disabled={disabled && !deferred}>
            {c.options?.map((o) => (
              <label key={o.value} className="flex items-center gap-2">
                <input
                  type="radio"
                  name={id}
                  checked={value === o.value}
                  required={c.required !== false}
                  onChange={() => choose(o.value)}
                />
                <OptionText o={o} />
              </label>
            ))}
          </fieldset>
        );
      return (
        <>
          <Select
            aria-label={
              c.placeholder ??
              c.label ??
              (c.type === T.Overflow ? t("apps.components.more") : undefined)
            }
            className={c.type === T.Overflow ? "w-auto" : "w-full max-w-xs"}
            multiple={many}
            required={labelled && c.required === true}
            disabled={disabled && !deferred}
            value={
              many
                ? ((value as string[]) ?? [])
                : c.type === T.Overflow
                  ? ""
                  : ((value as string) ?? "")
            }
            onChange={(e) => {
              const v = many ? [...e.target.selectedOptions].map((o) => o.value) : e.target.value;
              if (c.type === T.Overflow) choose(v);
              else choose(many ? v : [v as string].filter(Boolean));
            }}
          >
            {!many ? (
              <option value="">{c.type === T.Overflow ? "…" : (c.placeholder ?? "")}</option>
            ) : null}
            {c.options?.map((o) => (
              <option key={o.value} value={o.value} title={o.description}>
                {[o.emoji?.name, o.label].filter(Boolean).join(" ")}
              </option>
            ))}
          </Select>
        </>
      );
    }
    case T.CheckboxGroup:
      return <CheckList c={c} disabled={disabled && !deferred} choose={choose} value={value} />;
    case T.Checkbox:
      return (
        <input
          type="checkbox"
          disabled={disabled && !deferred}
          checked={!!value}
          onChange={(e) => choose(e.target.checked)}
        />
      );
    case T.TextInput: {
      const common = {
        value: (value as string) ?? "",
        required: c.required !== false,
        placeholder: c.placeholder,
        minLength: c.min_length,
        maxLength: c.max_length ?? 4000,
        disabled: disabled && !deferred,
        onChange: (e: { target: { value: string } }) => ctx.set(id, e.target.value),
      };
      if (c.style === 2 || c.format === "rich") return <Textarea {...common} />;
      return (
        <Input
          {...common}
          type={
            c.format === "number"
              ? "number"
              : c.format === "email"
                ? "email"
                : c.format === "url"
                  ? "url"
                  : "text"
          }
          min={c.min_value}
          max={c.max_value}
          step={c.format === "number" && !c.decimal ? 1 : undefined}
        />
      );
    }
    case T.DatePicker:
    case T.TimePicker:
    case T.DateTimePicker: {
      const type =
        c.type === T.DatePicker ? "date" : c.type === T.TimePicker ? "time" : "datetime-local";
      // datetime は UNIX 秒でやりとりし、入力欄では端末のタイムゾーンで表示する
      const shown =
        c.type === T.DateTimePicker && value
          ? new Date(Number(value) * 1000 - new Date().getTimezoneOffset() * 60000)
              .toISOString()
              .slice(0, 16)
          : ((value as string) ?? "");
      const toValue = (v: string) =>
        c.type === T.DateTimePicker && v ? String(Math.floor(new Date(v).getTime() / 1000)) : v;
      // 入力の途中（年を 1 桁打つたびなど）では送らない。メッセージ上では「送信」を押したときに 1 回だけ送る。
      // 送信中も入力欄は無効にしない（打っている途中の文字とフォーカスを失わないため）
      const field = (
        <Input
          type={type}
          className="w-auto"
          value={shown}
          required={c.required === true}
          disabled={(!!c.disabled || !ctx.appId) && !deferred}
          onChange={(e) => ctx.set(id, toValue(e.target.value))}
        />
      );
      if (deferred) return field;
      return (
        <span className="flex items-center gap-2">
          {field}
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={disabled || !value}
            onClick={() => confirm.run(() => void ctx.invoke("action", id, value))}
          >
            {t("apps.submit")}
          </Button>
        </span>
      );
    }
    case T.UserSelect:
    case T.ChannelSelect:
    case T.MentionableSelect:
    case T.RoleSelect:
      return <EntitySelect c={c} choose={choose} disabled={disabled && !deferred} />;
    case T.ExternalSelect:
      return (
        <ExternalSelect
          c={c}
          choose={choose}
          disabled={(!!c.disabled || !ctx.appId) && !deferred}
        />
      );
    case T.FileUpload:
      return <FileUpload c={c} disabled={disabled} />;
    default:
      return null;
  }
}

/** 複数を選ぶ select と checkbox group。選んだ値は配列で送る。 */
function CheckList({
  c,
  disabled,
  choose,
  value,
}: {
  c: Component;
  disabled: boolean;
  choose: (v: Value) => void;
  value: Value | undefined;
}) {
  const list = Array.isArray(value) ? value : [];
  return (
    <fieldset className="flex flex-col gap-1" aria-label={c.placeholder ?? c.label}>
      {c.options?.map((o) => (
        <label key={o.value} className="flex items-center gap-2">
          <input
            type="checkbox"
            disabled={disabled}
            checked={list.includes(o.value)}
            onChange={(e) =>
              choose(e.target.checked ? [...list, o.value] : list.filter((v) => v !== o.value))
            }
          />
          <OptionText o={o} />
        </label>
      ))}
    </fieldset>
  );
}

function OptionText({ o }: { o: SelectOption }) {
  return (
    <span className="flex flex-col">
      <span>{[o.emoji?.name, o.label].filter(Boolean).join(" ")}</span>
      {o.description ? (
        <span className="text-xs text-muted-foreground">{o.description}</span>
      ) : null}
    </span>
  );
}

/** user / channel select。zoo のユーザーとチャンネルから選ぶ（共有の collection を使い、別に取り直さない）。 */
function EntitySelect({
  c,
  choose,
  disabled,
}: {
  c: Component;
  choose: (v: Value) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const collections = useCollections();
  const ctx = useCtx();
  const users = useLiveQuery((q) => q.from({ users: collections.users }))?.data ?? [];
  const channels = useLiveQuery((q) => q.from({ channels: collections.channels }))?.data ?? [];
  const many = (c.max_values ?? 1) > 1;
  // zoo にはロールがないので、role select は選べない状態で出す。mentionable select はユーザーだけを候補にする
  if (c.type === T.RoleSelect)
    return (
      <span className="text-xs text-muted-foreground">{t("apps.components.unsupported")}</span>
    );
  const options: { id: string; label: string }[] = [];
  if (c.type !== T.ChannelSelect)
    options.push(...users.map((u) => ({ id: u.id, label: `@${u.user_name}` })));
  if (c.type === T.ChannelSelect)
    options.push(
      ...channels
        .filter((ch) => !ch.deleted_at)
        .map((ch) => ({ id: ch.id, label: `#${ch.channel_name}` })),
    );
  const value = ctx.values[c.custom_id ?? ""];
  return (
    <Select
      aria-label={c.placeholder ?? t("apps.components.choose")}
      className="w-full max-w-xs"
      multiple={many}
      disabled={disabled}
      value={many ? ((value as string[]) ?? []) : (((value as string[]) ?? [])[0] ?? "")}
      onChange={(e) => choose([...e.target.selectedOptions].map((o) => o.value).filter(Boolean))}
    >
      {!many ? <option value="">{c.placeholder ?? ""}</option> : null}
      {options.slice(0, 500).map((o) => (
        <option key={o.id} value={o.id}>
          {o.label}
        </option>
      ))}
    </Select>
  );
}

/**
 * 入力に合わせてアプリに選択肢を問い合わせる select。入力が止まって 400ms 後に 1 回だけ問い合わせる
 * （入力が変わらなければ問い合わせない。応答は最大 3 秒待つ）。
 */
function ExternalSelect({
  c,
  choose,
  disabled,
}: {
  c: Component;
  choose: (v: Value) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const ctx = useCtx();
  const collections = useCollections();
  const client = useMemo(
    () => (collections?.config ? createConfiguredApiClient(collections.config) : null),
    [collections?.config],
  );
  const [query, setQuery] = useState("");
  const [options, setOptions] = useState<SelectOption[]>(c.initial_options ?? []);
  const latest = useRef("");
  useEffect(() => {
    if (query.length < (c.min_query_length ?? 3) || !client || !ctx.appId) return;
    latest.current = query;
    // 選択肢の問い合わせは、ボタン操作の invoke（木全体の busy・error を変える）とは別の要求にする
    const timer = setTimeout(async () => {
      try {
        const asked = await client.request<{ request_id?: string }>(
          apiPath`/api/apps/${ctx.appId!}/invoke`,
          {
            method: "POST",
            body: {
              kind: "options",
              surface_id: ctx.surfaceId,
              message_id: ctx.messageId,
              channel_id: ctx.channelId,
              action_id: c.custom_id,
              values: { query },
            },
          },
        );
        if (!asked?.request_id || latest.current !== query) return;
        const got = await client.get<{ options: SelectOption[] | null }>(
          apiPath`/api/apps/${ctx.appId!}/options/${asked.request_id}`,
        );
        if (latest.current === query && got.options) setOptions(got.options);
      } catch {
        /* 候補が出ないだけにする。入力を続ければ問い合わせ直す */
      }
    }, 400);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);
  return (
    <div className="flex max-w-xs flex-col gap-1">
      <Input
        value={query}
        disabled={disabled}
        placeholder={c.placeholder ?? t("apps.components.search")}
        onChange={(e) => setQuery(e.target.value)}
        // 検索欄の Enter でモーダルのフォームを送らない
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.nativeEvent.isComposing) e.preventDefault();
        }}
      />
      <Select
        aria-label={c.placeholder ?? t("apps.components.choose")}
        disabled={disabled || !options.length}
        value={((ctx.values[c.custom_id ?? ""] as string[]) ?? [])[0] ?? ""}
        onChange={(e) => choose([e.target.value].filter(Boolean))}
      >
        <option value="">{options.length ? "" : t("apps.components.noOptions")}</option>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </Select>
    </div>
  );
}

/** モーダルの中でファイルを上げる。チャンネルの添付として上げ、送信の値は添付の ID にする（アプリは送信後に取得できる）。 */
function FileUpload({ c, disabled }: { c: Component; disabled: boolean }) {
  const { t } = useTranslation();
  const ctx = useCtx();
  const collections = useCollections();
  // アップロード処理は使うときにだけ読み込む（部品を描画するだけの画面では要らない）
  const upload = async (file: File) => {
    if (!collections?.config || !ctx.channelId) throw new Error("no channel");
    const { uploadAttachment } = await import("../../../lib/api/attachments");
    return uploadAttachment(createConfiguredApiClient(collections.config), ctx.channelId, file);
  };
  const [state, setState] = useState<"idle" | "uploading" | "error">("idle");
  const accept = (c.file_types ?? [])
    .map((x) => (x === "image" || x === "video" || x === "audio" ? `${x}/*` : x))
    .join(",");
  return (
    <div className="flex flex-col gap-1">
      <input
        type="file"
        multiple={(c.max_values ?? 1) > 1}
        accept={accept || undefined}
        required={c.required !== false}
        disabled={(disabled && !ctx.submit) || !ctx.channelId || state === "uploading"}
        onChange={async (e) => {
          const files = [...(e.target.files ?? [])].slice(0, c.max_values ?? 1);
          setState("uploading");
          try {
            const uploaded = await Promise.all(files.map((file) => upload(file)));
            ctx.set(
              c.custom_id ?? "",
              uploaded.map((a) => a.id),
            );
            setState("idle");
          } catch {
            setState("error");
          }
        }}
      />
      {state === "uploading" ? (
        <span className="text-xs text-muted-foreground">{t("apps.components.uploading")}</span>
      ) : null}
      {state === "error" ? (
        <span role="alert" className="text-xs text-destructive">
          {t("apps.actionError")}
        </span>
      ) : null}
    </div>
  );
}
