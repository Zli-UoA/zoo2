export const PREVIEW_USERS = [
  { id: "00000000-0000-0000-0000-000000000001", label: "管理者" },
  { id: "00000000-0000-0000-0000-000000000002", label: "一般ユーザー A" },
  { id: "00000000-0000-0000-0000-000000000003", label: "一般ユーザー B" },
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
