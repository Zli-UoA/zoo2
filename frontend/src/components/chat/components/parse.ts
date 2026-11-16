/**
 * メッセージ部品（Discord Components V2 形式、docs/apps.md「メッセージ部品」）の型と読み取り。
 *
 * サーバーが検証した文書（`messages.components`・`app_ui.document`・展開の `components`）を読む。
 * 形が崩れていれば null を返し、呼び出し側は本文を表示する。知らない type は「未対応の部品」として扱う。
 */

export const IS_COMPONENTS_V2 = 1 << 15;

export const T = {
  ActionRow: 1,
  Button: 2,
  StringSelect: 3,
  TextInput: 4,
  UserSelect: 5,
  RoleSelect: 6,
  MentionableSelect: 7,
  ChannelSelect: 8,
  Section: 9,
  TextDisplay: 10,
  Thumbnail: 11,
  MediaGallery: 12,
  File: 13,
  Separator: 14,
  Container: 17,
  Label: 18,
  FileUpload: 19,
  RadioGroup: 21,
  CheckboxGroup: 22,
  Checkbox: 23,
  Unsupported: 1000,
  ExternalSelect: 1001,
  Overflow: 1002,
  DatePicker: 1003,
  TimePicker: 1004,
  DateTimePicker: 1005,
  Video: 1008,
  Table: 1009,
  Alert: 1010,
  Card: 1011,
  Carousel: 1012,
  Chart: 1013,
  Grid: 1014,
  Plan: 1015,
  TaskCard: 1016,
} as const;

export interface Media {
  url: string;
}
export interface SelectOption {
  label: string;
  value: string;
  description?: string;
  default?: boolean;
  emoji?: { name?: string };
}
export interface Confirm {
  title: string;
  text: string;
  confirm: string;
  deny: string;
  style?: string;
}
export interface TableCell {
  text: string;
  number?: number;
  button?: Component;
}
export interface Task {
  task_id: string;
  title: string;
  status: string;
  details?: string;
  output?: string;
  sources?: { url: string; text: string }[];
}

/** 部品 1 つ。フィールドは Discord と同じ名前で、使う型ごとに一部だけを持つ。 */
export interface Component {
  type: number;
  id?: number;
  custom_id?: string;
  components?: Component[];
  accessory?: Component;
  component?: Component;
  content?: string;
  label?: string;
  description?: string;
  style?: number;
  url?: string;
  value?: string;
  disabled?: boolean;
  confirm?: Confirm;
  emoji?: { name?: string };
  options?: SelectOption[];
  placeholder?: string;
  min_values?: number;
  max_values?: number;
  required?: boolean;
  default_values?: { id: string; type: string }[];
  min_length?: number;
  max_length?: number;
  format?: "plain" | "number" | "email" | "url" | "rich";
  min_value?: number;
  max_value?: number;
  decimal?: boolean;
  media?: Media;
  spoiler?: boolean;
  items?: { media: Media; description?: string; spoiler?: boolean }[];
  file?: Media;
  name?: string;
  divider?: boolean;
  spacing?: number;
  accent_color?: number;
  title?: string;
  subtitle?: string;
  collapsible?: boolean;
  collapsed?: boolean;
  file_types?: string[];
  initial_value?: string;
  initial_options?: SelectOption[];
  min_query_length?: number;
  timezone?: string;
  video_url?: string;
  thumbnail_url?: string;
  title_url?: string;
  alt_text?: string;
  provider_name?: string;
  author_name?: string;
  rows?: TableCell[][] | { width?: number; align?: string; component: Component }[][];
  column_settings?: ({ align?: string; is_wrapped?: boolean } | null)[];
  header?: boolean;
  caption?: string;
  page_size?: number;
  level?: string;
  hero_image?: Media;
  icon?: Media;
  body?: string;
  subtext?: string;
  actions?: Component[];
  cards?: Component[];
  chart_type?: string;
  segments?: { label: string; value: number }[];
  series?: { name: string; data: { label: string; value: number }[] }[];
  categories?: string[];
  x_label?: string;
  y_label?: string;
  grid_lines?: string;
  tasks?: Task[];
  task_id?: string;
  status?: string;
  details?: string;
  output?: string;
  sources?: { url: string; text: string }[];
  original_type?: string;
  text?: string;
}

export interface ComponentDocument {
  flags: number;
  components: Component[];
}

const NAMES: Record<string, number> = Object.fromEntries(
  Object.entries({
    action_row: 1,
    button: 2,
    string_select: 3,
    text_input: 4,
    user_select: 5,
    role_select: 6,
    mentionable_select: 7,
    channel_select: 8,
    section: 9,
    text_display: 10,
    thumbnail: 11,
    media_gallery: 12,
    file: 13,
    separator: 14,
    container: 17,
    label: 18,
    file_upload: 19,
    radio_group: 21,
    checkbox_group: 22,
    checkbox: 23,
  }),
);

const MAX_DEPTH = 8;

function normalize(value: unknown, depth: number): Component | null {
  if (!value || typeof value !== "object" || Array.isArray(value) || depth > MAX_DEPTH) return null;
  const raw = value as Record<string, unknown>;
  const type =
    typeof raw.type === "number" ? raw.type : typeof raw.type === "string" ? NAMES[raw.type] : NaN;
  if (!Number.isInteger(type)) return null;
  const c = { ...raw, type } as Component;
  const list = (v: unknown) =>
    Array.isArray(v) ? v.map((x) => normalize(x, depth + 1)).filter((x): x is Component => !!x) : undefined;
  if (raw.components !== undefined) c.components = list(raw.components);
  if (raw.accessory !== undefined) c.accessory = normalize(raw.accessory, depth + 1) ?? undefined;
  if (raw.component !== undefined) c.component = normalize(raw.component, depth + 1) ?? undefined;
  if (raw.actions !== undefined) c.actions = list(raw.actions);
  if (raw.cards !== undefined) c.cards = list(raw.cards);
  return c;
}

/** 保存した JSON（文字列か値）を読む。読めなければ null。 */
export function parseComponents(value: unknown): ComponentDocument | null {
  let v = value;
  if (typeof v === "string") {
    if (!v.trim()) return null;
    try {
      v = JSON.parse(v);
    } catch {
      return null;
    }
  }
  if (!v || typeof v !== "object" || Array.isArray(v)) return null;
  const raw = v as Record<string, unknown>;
  if (!Array.isArray(raw.components) || raw.components.length > 500) return null;
  const components = raw.components
    .map((c) => normalize(c, 0))
    .filter((c): c is Component => !!c);
  if (!components.length) return null;
  return { flags: typeof raw.flags === "number" ? raw.flags : 0, components };
}

/** 本文を隠して部品だけを表示するか。 */
export function hidesContent(doc: ComponentDocument | null): boolean {
  return !!doc && (doc.flags & IS_COMPONENTS_V2) !== 0;
}

/** https の URL だけを通す（画像・リンク・動画）。 */
export function safeUrl(url: unknown): string | null {
  if (typeof url !== "string") return null;
  try {
    const u = new URL(url);
    return u.protocol === "https:" && !u.username && !u.password ? u.href : null;
  } catch {
    return null;
  }
}

/** `0xRRGGBB` を CSS の色にする。 */
export function accentColor(value: unknown): string | undefined {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 0xffffff
    ? `#${value.toString(16).padStart(6, "0")}`
    : undefined;
}

/** 入力値を持つ部品か（モーダルの送信で値を集める）。 */
export function isInput(c: Component): boolean {
  return (
    [T.TextInput, T.StringSelect, T.UserSelect, T.RoleSelect, T.MentionableSelect, T.ChannelSelect]
      .concat([T.FileUpload, T.RadioGroup, T.CheckboxGroup, T.Checkbox, T.ExternalSelect])
      .concat([T.DatePicker, T.TimePicker, T.DateTimePicker] as number[])
      .includes(c.type as never) && !!c.custom_id
  );
}

/** 選択を配列で送る部品か（select・checkbox group）。 */
export function multiValue(c: Component): boolean {
  return (
    [T.StringSelect, T.UserSelect, T.RoleSelect, T.MentionableSelect, T.ChannelSelect, T.ExternalSelect, T.CheckboxGroup, T.FileUpload] as number[]
  ).includes(c.type);
}
