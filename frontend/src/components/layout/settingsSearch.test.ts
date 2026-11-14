import { expect, it } from "vite-plus/test";

import { normalizeSettingsSearch } from "./settingsSearch";

it("古い「端末」タブは、暗号化タブの端末のセクションに直す", () => {
  expect(normalizeSettingsSearch({ tab: "devices" })).toMatchObject({
    tab: "encryption",
    section: "devices",
  });
});

it("知らないタブは一般タブにし、ほかの値は文字列だけを残す", () => {
  expect(normalizeSettingsSearch({ tab: "nope", channel: "c1", slack: 1 })).toMatchObject({
    tab: "general",
    section: undefined,
    channel: "c1",
    slack: undefined,
  });
});

it("暗号化タブの端末のセクションを指せる", () => {
  expect(normalizeSettingsSearch({ tab: "encryption", section: "devices" })).toMatchObject({
    tab: "encryption",
    section: "devices",
  });
});
