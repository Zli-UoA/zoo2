import { afterEach, expect, it, vi } from "vite-plus/test";
import { DEFAULT_CONFIG } from "../config";
import { applicationServerKey, disablePush } from "./push";

vi.mock("../api/client", () => ({ createConfiguredApiClient: () => ({ request }) }));
const { request } = vi.hoisted(() => ({ request: vi.fn() }));
afterEach(() => {
  vi.unstubAllGlobals();
  request.mockReset();
});

it("decodes URL-safe VAPID bytes without padding", () => {
  expect([...applicationServerKey("-_8-")]).toEqual([251, 255, 62]);
});

it("revokes browser delivery before deleting the server subscription", async () => {
  const order: string[] = [];
  const unsubscribe = vi.fn(async () => {
    order.push("local");
    return true;
  });
  vi.stubGlobal("navigator", {
    serviceWorker: {
      getRegistration: async () => ({
        pushManager: {
          getSubscription: async () => ({
            endpoint: "https://fcm.googleapis.com/test",
            unsubscribe,
          }),
        },
      }),
    },
  });
  request.mockImplementation(async () => {
    order.push("server");
    throw new Error("offline");
  });
  await expect(disablePush(DEFAULT_CONFIG)).rejects.toThrow("offline");
  expect(order).toEqual(["local", "server"]);
  expect(request).toHaveBeenCalledWith("/api/push/subscriptions", {
    method: "DELETE",
    body: { endpoint: "https://fcm.googleapis.com/test" },
  });
});

it("reports local revocation failure and keeps the server registration", async () => {
  vi.stubGlobal("navigator", {
    serviceWorker: {
      getRegistration: async () => ({
        pushManager: {
          getSubscription: async () => ({
            endpoint: "https://fcm.googleapis.com/test",
            unsubscribe: async () => false,
          }),
        },
      }),
    },
  });
  await expect(disablePush(DEFAULT_CONFIG)).rejects.toThrow("Could not revoke");
  expect(request).not.toHaveBeenCalled();
});

it("does nothing for a browser without a subscription", async () => {
  vi.stubGlobal("navigator", { serviceWorker: { getRegistration: async () => undefined } });
  await disablePush(DEFAULT_CONFIG);
  expect(request).not.toHaveBeenCalled();
});
