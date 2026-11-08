/** Remove private data left on devices by the retired Compose Web client. */
export function clearLegacyFrontendStorage(userId: string | null): void {
  if (!userId) return;
  const prefix = `zoo-compose:${userId}:`;
  try {
    for (const key of Object.keys(localStorage))
      if (key.startsWith(prefix)) localStorage.removeItem(key);
  } catch {
    // Blocked storage is inaccessible; OPFS deletion must still be attempted.
  }
}
