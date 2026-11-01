import { fn } from "storybook/test";

export const searchDemoQuery = "猫 café & zoo";
export const longSearchDemoQuery = "長い検索語abcdefgh ".repeat(25).trimEnd();
export const blockedPushLabel = "ブラウザーの設定で通知を許可してください";

export const pushStoryControls = {
  label: "プッシュ通知を有効にする",
  enabled: false,
  busy: false,
  supported: true,
  publicKey: "storybook-key",
  blocked: false,
  toggle: fn(async () => {}),
};

export const pushStoryPresets = {
  "Select Off": pushStoryControls,
  "Select On": { ...pushStoryControls, enabled: true },
  "Select Busy": { ...pushStoryControls, busy: true },
  "Select Blocked": { ...pushStoryControls, blocked: true, label: blockedPushLabel },
};

export const channelRuleInitialPattern = "ltl-%";
export const channelRuleSavedPattern = "public-%";
