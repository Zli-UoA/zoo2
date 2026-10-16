import { request } from "@playwright/test";

/**
 * スモークテストの前提を整えます。現在ユーザー（`VITE_DEV_USER_ID`）が
 * バックエンドにまだ存在しなければ、`POST /api/users` で作成します。
 * チャンネル作成は作成者の FK を持つため、このユーザーが必要です。
 */
export default async function globalSetup(): Promise<void> {
  const apiBaseUrl = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
  const devUserId = process.env.VITE_DEV_USER_ID ?? "00000000-0000-0000-0000-000000000001";

  const context = await request.newContext();
  try {
    const existing = await context.get(`${apiBaseUrl}/api/users/${devUserId}`);
    if (existing.status() === 404) {
      const created = await context.post(`${apiBaseUrl}/api/users`, {
        data: {
          id: devUserId,
          user_name: `smoke-${devUserId.slice(0, 8)}`,
          display_name: "Smoke User",
        },
      });
      if (!created.ok()) {
        throw new Error(`failed to seed dev user ${devUserId}: ${created.status()}`);
      }
    } else if (!existing.ok()) {
      throw new Error(`failed to read dev user ${devUserId}: ${existing.status()}`);
    }
  } finally {
    await context.dispose();
  }
}
