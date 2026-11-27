export const PREVIEW_USERS = [
  { id: "00000000-0000-0000-0000-000000000001", role: "admin", name: "" },
  { id: "00000000-0000-0000-0000-000000000002", role: "member", name: "A" },
  { id: "00000000-0000-0000-0000-000000000003", role: "member", name: "B" },
] as const;

export const PREVIEW_USER_KEY = "zoo2-preview-user";

export function previewUserId(): string {
  try {
    const selected = window.sessionStorage.getItem(PREVIEW_USER_KEY);
    return PREVIEW_USERS.find((user) => user.id === selected)?.id ?? PREVIEW_USERS[0].id;
  } catch {
    return PREVIEW_USERS[0].id;
  }
}
