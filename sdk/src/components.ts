/**
 * Message components (Discord Components V2 shape). See docs/apps.md「メッセージ部品」.
 *
 * Stored and returned with numeric `type`. Zoo accepts Discord names (`"container"`) as input too.
 * `text_display.content` is Zoo MFM: use {@link plain} for untrusted text.
 */
export const IS_COMPONENTS_V2 = 1 << 15;

export const ComponentType = {
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

export const ButtonStyle = { Primary: 1, Secondary: 2, Success: 3, Danger: 4, Link: 5 } as const;

export interface Media { url: string; width?: number; height?: number; content_type?: string }
export interface Emoji { id?: string; name?: string; animated?: boolean }
export interface Confirm { title: string; text: string; confirm: string; deny: string; style?: "primary" | "danger" }
export interface SelectOption { label: string; value: string; description?: string; emoji?: Emoji; default?: boolean }

/** Any component. Fields follow Discord; Zoo-only components use type numbers 1000+. */
export interface Component {
  type: number | string;
  id?: number;
  [key: string]: unknown;
}

export interface ComponentDocument { flags?: number; components: Component[] }

/** Wrap text so MFM does not interpret it (`*`, `@`, `$[` and so on). */
export function plain(text: string): string {
  if (!text) return "";
  if (!/[*_~`[\]()<>#$@:\\|>\-!?\n]/.test(text)) return text;
  return `<plain>${text.replaceAll("</plain>", "</plain><plain></</plain><plain>plain>")}</plain>`;
}

/** `#rrggbb` → integer accent colour. */
export function color(hex: string): number {
  return Number.parseInt(hex.replace(/^#/, ""), 16);
}

export const textDisplay = (content: string): Component => ({ type: 10, content });
export const separator = (divider = true, spacing: 1 | 2 = 1): Component => ({ type: 14, divider, spacing });
export const container = (components: Component[], accent_color?: number | string): Component => ({
  type: 17,
  ...(accent_color === undefined ? {} : { accent_color: typeof accent_color === "string" ? color(accent_color) : accent_color }),
  components,
});
export const section = (texts: string[], accessory: Component): Component => ({ type: 9, components: texts.map(textDisplay), accessory });
export const thumbnail = (url: string, description?: string): Component => ({ type: 11, media: { url }, ...(description ? { description } : {}) });
export const gallery = (items: { url: string; description?: string }[]): Component => ({
  type: 12,
  items: items.map(({ url, description }) => ({ media: { url }, ...(description ? { description } : {}) })),
});
export const actionRow = (components: Component[]): Component => ({ type: 1, components });
export const button = (label: string, custom_id: string, options: { style?: 1 | 2 | 3 | 4; value?: string; disabled?: boolean; confirm?: Confirm } = {}): Component => ({
  type: 2,
  style: options.style ?? 2,
  label,
  custom_id,
  ...(options.value === undefined ? {} : { value: options.value }),
  ...(options.disabled ? { disabled: true } : {}),
  ...(options.confirm ? { confirm: options.confirm } : {}),
});
export const linkButton = (label: string, url: string): Component => ({ type: 2, style: 5, label, url });
export const stringSelect = (custom_id: string, options: SelectOption[], extra: { placeholder?: string; min_values?: number; max_values?: number } = {}): Component => ({ type: 3, custom_id, options, ...extra });
export const label = (text: string, component: Component, description?: string): Component => ({ type: 18, label: text, component, ...(description ? { description } : {}) });
export const textInput = (custom_id: string, options: { style?: 1 | 2; required?: boolean; placeholder?: string; value?: string; min_length?: number; max_length?: number; format?: "plain" | "number" | "email" | "url" | "rich" } = {}): Component => ({ type: 4, custom_id, style: options.style ?? 1, ...options });

/** A message that shows only its components (the body becomes the notification fallback). */
export const message = (components: Component[]): ComponentDocument => ({ flags: IS_COMPONENTS_V2, components });
