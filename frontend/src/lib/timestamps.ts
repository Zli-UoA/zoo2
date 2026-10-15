export type TimestampString = string;

const TIMESTAMP_PATTERN = /^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2}(?:\.\d+)?)$/;

export function normalizeUtcTimestamp(value: TimestampString): string {
  const match = TIMESTAMP_PATTERN.exec(value);
  if (!match) {
    return value;
  }
  return `${match[1]}T${match[2]}Z`;
}

export function parseUtcTimestamp(value: TimestampString): Date {
  return new Date(normalizeUtcTimestamp(value));
}
