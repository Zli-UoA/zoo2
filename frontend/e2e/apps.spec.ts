import { expect, test } from "@playwright/test";

test.use({ locale: "ja-JP" });
test("Appを作成・承認し、Botとして投稿と個人宛応答を表示する", async ({ page, request }) => {
  const api = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
  const actor = process.env.VITE_DEV_USER_ID ?? "00000000-0000-0000-0000-000000000001";
  const name = `Echo ${Date.now()}`;
  await page.goto("/");
  await page.getByRole("link", { name: "Zoo設定", exact: true }).click();
  await page.getByRole("tab", { name: "App・連携", exact: true }).click();
  await page.getByRole("button", { name: "Appを作成", exact: true }).click();
  await page.getByLabel("App名", { exact: true }).fill(name);
  await page.getByLabel("説明", { exact: true }).fill("App platform end-to-end test");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await page.getByRole("button", { name: "承認を申請", exact: true }).click();
  await page.getByRole("button", { name: "承認してインストール", exact: true }).click();
  await expect(page.getByText("インストール済み", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "トークンを発行", exact: true }).click();
  const credentials = JSON.parse(await page.getByLabel(/認証情報/).inputValue()) as {
    access_token: string;
  };
  const authHeaders = { Authorization: `Bearer ${credentials.access_token}` };
  const auth = await request.get(`${api}/api/apps/v1/auth`, { headers: authHeaders });
  expect(auth.ok()).toBe(true);
  const identity = (await auth.json()) as { app_id: string; user_id: string };
  const channelResponse = await request.post(`${api}/api/channels`, {
    headers: { "X-Dev-User": actor },
    data: { channel_name: `app-e2e-${Date.now()}`, is_private: true },
  });
  expect(channelResponse.ok()).toBe(true);
  const channel = (await channelResponse.json()).data as { id: string };
  await page.getByRole("button", { name: "チャンネル一覧へ戻る" }).click();
  await page.goto(`/channels/${channel.id}`);
  await page.getByRole("link", { name: "Zoo設定", exact: true }).click();
  await page.getByRole("tab", { name: "App・連携", exact: true }).click();
  await page.getByRole("button", { name: new RegExp(name) }).click();
  const addChannel = page.getByRole("button", { name: "このチャンネルに追加", exact: true });
  await addChannel.click();
  await expect(addChannel).toBeEnabled();
  await page.getByRole("button", { name: "チャンネル一覧へ戻る" }).click();
  const content = `Bot message ${Date.now()}`;
  const posted = await request.post(`${api}/api/apps/v1/messages`, {
    headers: authHeaders,
    data: { channel_id: channel.id, content },
  });
  expect(posted.ok(), await posted.text()).toBe(true);
  await expect(page.getByText(content, { exact: true })).toBeVisible({ timeout: 15000 });
  await expect(page.getByText("APP", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: new RegExp(name) }).click();
  await expect(page.getByRole("heading", { name: new RegExp(name) })).toBeVisible();
  await page.keyboard.press("Escape");
  const privateText = `Private response ${Date.now()}`;
  const surface = await request.post(`${api}/api/apps/v1/surfaces`, {
    headers: authHeaders,
    data: {
      kind: "ephemeral",
      user_id: actor,
      channel_id: channel.id,
      blocks: [{ type: "text", text: privateText }],
    },
  });
  expect(surface.ok()).toBe(true);
  await expect(page.getByText(privateText, { exact: true })).toBeVisible({ timeout: 10000 });
  await expect(page.getByText(`${name} · 自分にのみ表示`, { exact: true })).toBeVisible();
  // The app identity remains excluded from the human user API.
  const users = await request.get(`${api}/api/users`, { headers: { "X-Dev-User": actor } });
  expect(
    ((await users.json()) as { id: string }[]).some((user) => user.id === identity.user_id),
  ).toBe(false);
});

test("Appの外部接続要求とAssistantの開始・改名・停止を操作できる", async ({ page, request }) => {
  const api = process.env.VITE_API_BASE_URL ?? "http://localhost:8000";
  const actor = process.env.VITE_DEV_USER_ID ?? "00000000-0000-0000-0000-000000000001";
  const headers = { "X-Dev-User": actor };
  const name = `Assistant ${Date.now()}`;
  const created = await request.post(`${api}/api/apps`, {
    headers,
    data: {
      name,
      manifest: {
        scopes: ["users:read", "users:write", "assistant:write"],
        events: [
          "user.connection_requested",
          "assistant.started",
          "assistant.title_changed",
          "assistant.stopped",
          "app.context_changed",
        ],
        transport: "websocket",
        user_connections: true,
      },
    },
  });
  expect(created.ok()).toBe(true);
  const app = (await created.json()) as { id: string };
  expect((await request.post(`${api}/api/apps/${app.id}/actions/submit`, { headers })).ok()).toBe(
    true,
  );
  expect(
    (
      await request.post(`${api}/api/apps/${app.id}/actions/install`, {
        headers,
        data: { revision: 1 },
      })
    ).ok(),
  ).toBe(true);
  const credentials = (await (
    await request.post(`${api}/api/apps/${app.id}/actions/token`, { headers })
  ).json()) as { access_token: string };
  const channel = (
    await (
      await request.post(`${api}/api/channels`, {
        headers,
        data: { channel_name: `assistant-${Date.now()}`, is_private: true },
      })
    ).json()
  ).data as { id: string };
  expect(
    (await request.post(`${api}/api/apps/${app.id}/channels/${channel.id}`, { headers })).ok(),
  ).toBe(true);
  await page.goto(`/channels/${channel.id}`);
  await page.getByRole("link", { name: "Zoo設定", exact: true }).click();
  await page.getByRole("tab", { name: "App・連携", exact: true }).click();
  await page.getByRole("button", { name: new RegExp(name) }).click();
  await page.getByRole("button", { name: "アカウントを接続", exact: true }).click();
  await expect(page.getByText("接続を要求済み", { exact: true })).toBeVisible();
  expect(
    (
      await request.post(`${api}/api/apps/v1/user-connections/${actor}`, {
        headers: { Authorization: `Bearer ${credentials.access_token}` },
        data: { connected: true },
      })
    ).ok(),
  ).toBe(true);
  await expect(page.getByText("接続済み", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "アシスタントを開始", exact: true }).click();
  const title = page.getByRole("textbox", { name: "セッション名" });
  await expect(title).toBeVisible();
  await title.fill("会話のまとめ");
  await title.press("Tab");
  await expect
    .poll(async () => {
      const sessions = (await (
        await request.get(`${api}/api/assistant-sessions`, { headers })
      ).json()) as { app_id: string; title: string }[];
      return sessions.find((session) => session.app_id === app.id)?.title;
    })
    .toBe("会話のまとめ");
  await page.getByRole("button", { name: "停止", exact: true }).click();
  await expect(title).not.toBeVisible();
  await page.getByRole("button", { name: "接続を解除", exact: true }).click();
  await expect(page.getByText("未接続", { exact: true })).toBeVisible();
  const deliveries = (await (
    await request.get(`${api}/api/apps/${app.id}/deliveries`, { headers })
  ).json()) as { type: string }[];
  for (const type of [
    "user.connection_requested",
    "assistant.started",
    "assistant.title_changed",
    "assistant.stopped",
    "app.context_changed",
  ])
    expect(deliveries.some((delivery) => delivery.type === type)).toBe(true);
});
