import { expect, test } from "@playwright/test";
import { fillControl, expectControlValue } from "./compose-control-input";

const api = "https://127.0.0.1:4197";
for (const implementation of ["react", "kotlin"] as const)
  for (const scenario of [
    { name: "name", value: "my-channel-name", patch: { channel_name: "my-channel-name" } },
    { name: "description", value: "My description", patch: { description: "My description" } },
    { name: "clear description", value: "", patch: { description: "" } },
  ])
    test(`${implementation}: rename ${scenario.name} patches only edited fields after an external Electric update`, async ({
      page,
      request,
    }) => {
      await request.post(`${api}/__reset`);
      await page.addInitScript(() => {
        localStorage.setItem("zoo-locale", "ja");
        localStorage.setItem("zoo-theme", "light");
      });
      await page.goto(`/channels/c1?frontend=${implementation === "kotlin" ? "compose" : "react"}`);
      await expect(page.getByText("こんにちは Parity", { exact: false })).toBeVisible({
        timeout: 90_000,
      });
      const native = implementation === "kotlin";
      await page
        .getByRole("button", { name: "チャンネル設定", exact: true })
        .click({ force: native });
      await page
        .getByRole("dialog", { name: "チャンネル設定", exact: true })
        .getByRole("button", { name: "編集", exact: true })
        .click({ force: native });
      const dialog = page.getByRole("dialog", { name: "チャンネルを編集", exact: true });
      await expect(dialog).toBeVisible();
      const name = dialog.getByRole("textbox", { name: "新しいチャンネル名", exact: true });
      const description = dialog.getByRole("textbox", { name: "説明（任意）", exact: true });
      await expectControlValue(name, "general", implementation);
      const external = {
        channel_name: "externally-renamed",
        description: "External description",
        is_private: true,
      };
      const updated = await request.patch(`${api}/api/channels/c1`, { data: external });
      expect(updated.ok()).toBe(true);
      // Observe the real subscribed UI, even though the modal hides its header from accessibility.
      await expect(
        page.getByRole("heading", {
          name: external.channel_name,
          exact: true,
          includeHidden: true,
        }),
      ).toBeAttached({ timeout: 30_000 });
      // The editing baseline remains the value captured when the dialog opened.
      await expectControlValue(name, "general", implementation);
      await expectControlValue(description, "共通の画面と操作", implementation);
      await expect(dialog.getByRole("button", { name: "保存", exact: true })).toBeDisabled();
      const patches: unknown[] = [];
      page.on("request", (event) => {
        if (event.method() === "PATCH" && new URL(event.url()).pathname === "/api/channels/c1")
          patches.push(event.postDataJSON());
      });
      await fillControl(
        page,
        scenario.name === "name" ? name : description,
        scenario.value,
        implementation,
      );
      await dialog.getByRole("button", { name: "保存", exact: true }).click({ force: native });
      await expect.poll(() => patches).toEqual([scenario.patch]);
      await expect(dialog).toBeHidden();
      await expect
        .poll(async () => {
          const response = await request.get(`${api}/__state`);
          const state = (await response.json()) as {
            channels: {
              id: string;
              channel_name: string;
              description: string;
              is_private: boolean;
            }[];
          };
          return state.channels.find((channel) => channel.id === "c1");
        })
        .toMatchObject({ id: "c1", ...external, ...scenario.patch });
    });
