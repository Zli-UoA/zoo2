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

export const activityPubProfileInitial = {
  display_name: "Federated Alice",
  default_display_name: "Alice",
  summary: "Zooで開発しています",
};
export const activityPubProfileSaved = {
  display_name: "連合のアリス",
  summary: "公開プロフィールです",
};
export const activityPubProfileRejectedName = "保存前の名前";

export const customEmojiFixture = {
  width: 3,
  height: 1,
  name: "zoo_wave",
  url: "/api/emoji-images/30a12e81-bc91-4574-b89b-8b87c08c0884",
  category: "Zoo",
  aliases: ["hello", "挨拶"],
};

export const customEmojiImageBase64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=";
