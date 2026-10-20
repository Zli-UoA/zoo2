export const DEFAULT_TENANT_ID = "00000000-0000-0000-0000-000000000001";
const STORAGE_KEY = "zoo.tenantId";

export function getSelectedTenantId(): string {
  if (typeof window === "undefined") return DEFAULT_TENANT_ID;
  try {
    return window.sessionStorage.getItem(STORAGE_KEY) ?? DEFAULT_TENANT_ID;
  } catch {
    return DEFAULT_TENANT_ID;
  }
}

export function saveSelectedTenantId(id: string): boolean {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, id);
    return true;
  } catch {
    return false;
  }
}

export function switchTenant(id: string): void {
  if (!saveSelectedTenantId(id)) {
    throw new Error("Workspace switching requires session storage access");
  }
  // Dispose the entire page: subscriptions, optimistic writes, and channel routes
  // must never survive a workspace switch. sessionStorage keeps tabs independent.
  window.location.assign("/");
}
