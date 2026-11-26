export function proxyImageUrl(endpoint: string, image: string | null): string | null {
  if (!endpoint || !image) return null;
  try {
    const source = new URL(image);
    if (!/^https?:$/.test(source.protocol) || source.username || source.password) return null;
    // Relative endpoints must stay on this origin; protocol-relative URLs are rejected.
    const relative = endpoint.startsWith("/") && !endpoint.startsWith("//");
    const proxy = new URL(endpoint, relative ? "https://same-origin.invalid" : undefined);
    if (
      proxy.protocol !== "https:" &&
      !(proxy.protocol === "http:" && /^(localhost|127\.0\.0\.1)$/.test(proxy.hostname))
    )
      return null;
    if (proxy.username || proxy.password) return null;
    proxy.searchParams.set("url", source.href);
    // Use the proxy's configured output size instead of its smaller thumbnail presets.
    proxy.searchParams.delete("preview");
    proxy.searchParams.delete("static");
    return relative ? `${proxy.pathname}${proxy.search}` : proxy.href;
  } catch {
    return null;
  }
}
