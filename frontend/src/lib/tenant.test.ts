import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { createApiClient } from "./api/client";
import { createShapeFetch } from "./electric/headers";
import {
  DEFAULT_TENANT_ID,
  getSelectedTenantId,
  saveSelectedTenantId,
  switchTenant,
} from "./tenant";

afterEach(() => vi.unstubAllGlobals());

function browser() {
  const values = new Map<string, string>();
  const assign = vi.fn();
  vi.stubGlobal("window", {
    sessionStorage: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    },
    location: { assign },
  });
  return { assign };
}

describe("workspace selection", () => {
  it("defaults to the migrated workspace and saves the tab's selection", () => {
    browser();
    expect(getSelectedTenantId()).toBe(DEFAULT_TENANT_ID);
    saveSelectedTenantId("tenant-a");
    expect(getSelectedTenantId()).toBe("tenant-a");
  });

  it("navigates away from old channel routes when switching", () => {
    const { assign } = browser();
    switchTenant("tenant-b");
    expect(getSelectedTenantId()).toBe("tenant-b");
    expect(assign).toHaveBeenCalledWith("/");
  });

  it("pins REST and Electric requests to the workspace at client creation", async () => {
    browser();
    saveSelectedTenantId("tenant-a");
    const restHeaders: Headers[] = [];
    const shapeHeaders: Headers[] = [];
    const client = createApiClient({
      getToken: () => null,
      fetch: async (_input, init) => {
        restHeaders.push(new Headers(init?.headers));
        return new Response("[]", { status: 200 });
      },
    });
    const shape = createShapeFetch({
      getToken: () => null,
      fetch: async (_input, init) => {
        shapeHeaders.push(new Headers(init?.headers));
        return new Response("[]", { status: 200 });
      },
    });
    saveSelectedTenantId("tenant-b");
    await client.get("/api/channels");
    await shape("/api/shapes/channels");
    expect(restHeaders[0]?.get("X-Tenant-ID")).toBe("tenant-a");
    expect(shapeHeaders[0]?.get("X-Tenant-ID")).toBe("tenant-a");
  });

  it("omits tenant headers when using the development-only direct Electric connection", async () => {
    const captured: Headers[] = [];
    const shape = createShapeFetch({
      auth: false,
      fetch: async (_input, init) => {
        captured.push(new Headers(init?.headers));
        return new Response("[]");
      },
    });
    await shape("http://localhost:3000/v1/shape");
    expect(captured[0]?.has("X-Tenant-ID")).toBe(false);
  });
});
