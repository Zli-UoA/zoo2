export const DEFAULT_TENANT_ID = "00000000-0000-0000-0000-000000000001";
const STORAGE_KEY = "zoo.tenantId";

export function getSelectedTenantId(): string {
  if (typeof window === "undefined") return DEFAULT_TENANT_ID;
  return window.sessionStorage.getItem(STORAGE_KEY) ?? DEFAULT_TENANT_ID;
}

export function saveSelectedTenantId(id: string): void {
  window.sessionStorage.setItem(STORAGE_KEY, id);
}

export function switchTenant(id: string): void {
  saveSelectedTenantId(id);
  // Dispose the entire page: subscriptions, optimistic writes, and channel routes
  // must never survive a workspace switch. sessionStorage keeps tabs independent.
  window.location.assign("/");
}
