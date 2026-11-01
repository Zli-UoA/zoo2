import type { NativeInteraction } from "./native-interactions";
import {
  searchDemoQuery,
  channelRuleInitialPattern,
  activityPubProfileInitial,
  activityPubProfileSaved,
  activityPubProfileRejectedName,
  channelRuleSavedPattern,
  longSearchDemoQuery,
  pushStoryControls,
  pushStoryPresets,
} from "./preference-fixtures";
import type { KotlinPreviewProps } from "./KotlinPreview";
import type { Selection } from "./kotlin-stories";

export const preferenceSurfaces = {
  "Features/Workspace/Members": "mx-auto max-w-2xl p-4",
  "Features/Workspace/SlackImport": "mx-auto max-w-2xl p-4",
  "Features/Workspace/SlackLink": "mx-auto max-w-2xl p-4",
  "Flows/Workspace/ChangeRole": "mx-auto max-w-2xl p-4",
  "Features/Preferences/MfmSearch": "mx-auto max-w-xl space-y-4 p-4",
  "Features/Preferences/PWA": "flex min-h-72 items-center justify-center gap-2 bg-background p-6",
  "Features/Preferences/Theme": "flex min-h-72 items-center justify-center bg-background p-6",
  "Features/Preferences/Language": "flex min-h-72 items-center justify-center bg-background p-6",
} as const;

export function preferenceOptions(
  context: Selection,
  base: KotlinPreviewProps,
): KotlinPreviewProps | null {
  const { title, name, args, parameters } = context;
  if (!(title in preferenceSurfaces) && title !== "Pages/ZooSettings") return null;
  const kind =
    title === "Pages/ZooSettings"
      ? "settings"
      : title === "Features/Workspace/Members" || title === "Flows/Workspace/ChangeRole"
        ? "members"
        : title.split("/").at(-1)!.toLowerCase();
  const state = name;
  const pushState =
    kind === "pwa" && name in pushStoryPresets
      ? pushStoryPresets[name as keyof typeof pushStoryPresets]
      : undefined;
  type Step = NativeInteraction;
  const interactions: Step[] = [];
  const wait = (role: string, name: string, extra: Partial<Step> = {}) =>
    interactions.push({ action: "wait", role, name, exact: true, ...extra });
  const click = (role: string, name: string, extra: Partial<Step> = {}) =>
    interactions.push({ action: "click", role, name, exact: true, ...extra });
  const select = (name: string, value: string) =>
    interactions.push({ action: "select", role: "combobox", name, value, exact: true });
  if (kind === "settings") {
    if (name.startsWith("Apps") || name === "Management") {
      click("tab", "App・連携");
      click("button", "Echo Bot", { exact: false });
      if (name === "Management") {
        click("tab", "管理・開発");
        wait("button", "トークンを発行");
      } else wait("button", "メッセージ");
    } else if (name === "Members") {
      click("tab", "メンバー");
      wait("button", "管理者にする", { index: 0, enabled: true });
    } else if (name.startsWith("Activity Pub") || name.startsWith("Channel Rule")) {
      click("tab", "ActivityPub");
    } else if (name === "Slack Import") {
      click("tab", "Slack 取り込み");
      wait("button", "ZIP を選んでアップロード", { enabled: true });
    } else {
      wait("heading", "Zoo設定");
      wait("group", "テーマ");
      wait("combobox", "言語");
      wait("combobox", "検索エンジン", { enabled: true });
      wait("combobox", "プッシュ通知", { enabled: false });
    }
  }
  if (kind === "settings" && name === "Channel Rule") {
    wait("textbox", "LTLに流すチャンネル名のルール", {
      enabled: true,
      valueExact: channelRuleInitialPattern,
    });
    interactions.push({
      action: "type",
      role: "textbox",
      name: "LTLに流すチャンネル名のルール",
      value: channelRuleSavedPattern,
      exact: true,
    });
    click("button", "保存", { within: { role: "form", name: "LTLに流すチャンネル名のルール" } });
    wait("button", "保存", {
      enabled: true,
      within: { role: "form", name: "LTLに流すチャンネル名のルール" },
    });
    wait("textbox", "LTLに流すチャンネル名のルール", { valueExact: channelRuleSavedPattern });
  }
  if (kind === "settings" && name === "Channel Rule Error") {
    wait("alert", "", { exact: false });
    click("button", "更新");
    wait("textbox", "LTLに流すチャンネル名のルール", {
      enabled: true,
      valueExact: channelRuleInitialPattern,
    });
    click("button", "保存", { within: { role: "form", name: "LTLに流すチャンネル名のルール" } });
    wait("alert", "", { exact: false });
    wait("button", "保存", {
      enabled: true,
      within: { role: "form", name: "LTLに流すチャンネル名のルール" },
    });
  }
  if (kind === "settings" && name.startsWith("Activity Pub")) {
    const profileForm = { role: "form", name: "ActivityPub" };
    if (name === "Activity Pub Loading") wait("status", "", { exact: false, text: "読み込み中" });
    else if (name === "Activity Pub Error") wait("alert", "", { exact: false });
    else {
      wait("textbox", "APub表示名", { valueExact: activityPubProfileInitial.display_name });
      interactions.push({
        action: "type",
        role: "textbox",
        name: "APub表示名",
        value:
          name === "Activity Pub Save Error"
            ? activityPubProfileRejectedName
            : activityPubProfileSaved.display_name,
        exact: true,
      });
      if (name !== "Activity Pub Save Error")
        interactions.push({
          action: "type",
          role: "textbox",
          name: "自己紹介",
          value: activityPubProfileSaved.summary,
          exact: true,
        });
      click("button", "保存", { within: profileForm });
      if (name === "Activity Pub Save Error") {
        wait("alert", "", { exact: false });
        wait("textbox", "APub表示名", { valueExact: activityPubProfileRejectedName });
      } else {
        wait("status", "", { exact: false });
        interactions.push({
          action: "type",
          role: "textbox",
          name: "APub表示名",
          value: "",
          exact: true,
        });
        click("button", "保存", { within: profileForm });
        wait("textbox", "APub表示名", {
          valueExact: "",
          attributes: { "aria-placeholder": "Alice" },
        });
        wait("status", "", { exact: false });
      }
    }
  }
  if (kind === "members") {
    if (name === "Load Error") {
      wait("alert", "", { exact: false });
      wait("button", "再試行", { enabled: true });
    } else if (name !== "Loading") {
      if (name === "Search Empty")
        interactions.push({
          action: "type",
          role: "searchbox",
          name: "名前で検索して追加",
          value: "no-such-member",
          exact: true,
        });
      if (
        [
          "Admin",
          "Mobile",
          "Protected Admin",
          "Last Admin",
          "Grant And Revoke",
          "Rejected Last Admin",
        ].includes(name)
      )
        wait("button", name === "Rejected Last Admin" ? "管理者を外す" : "管理者にする", {
          index: 0,
          enabled: true,
        });
      else if (name === "Read Only") {
        interactions.push({ action: "wait", text: "@admin", exact: true });
        interactions.push({ action: "wait", text: "管理者", exact: true });
        const memberCount = Array.isArray(base.fixture?.users) ? base.fixture.users.length : 0;
        wait("listitem", "", { exact: false, count: memberCount });
        wait("button", "管理者にする", { state: "hidden" });
        wait("button", "管理者を外す", { state: "hidden" });
      } else if (name === "Empty" || name === "Search Empty") {
        interactions.push({ action: "wait", text: "該当するユーザーがいません。", exact: true });
        wait("listitem", "", { exact: false, state: "hidden" });
      }
      if (name === "Protected Admin") {
        wait("button", "管理者を外す", { within: { text: "@alice" }, enabled: true });
        wait("button", "管理者を外す", { within: { text: "@admin" }, enabled: false });
      }
      if (name === "Last Admin") {
        wait("button", "管理者にする", { within: { text: "@admin" }, enabled: true });
        wait("button", "管理者を外す", { within: { text: "@alice" }, enabled: false });
      }
      if (name === "Grant And Revoke") {
        click("button", "管理者にする", { within: { text: "@bob" } });
        click("button", "管理者にする", { within: { role: "alertdialog" } });
        wait("alertdialog", "", { exact: false, state: "hidden" });
        wait("button", "管理者を外す", { within: { text: "@bob" }, enabled: true });
        click("button", "管理者を外す", { within: { text: "@bob" } });
        click("button", "管理者を外す", { within: { role: "alertdialog" } });
        wait("alertdialog", "", { exact: false, state: "hidden" });
        wait("button", "管理者にする", { within: { text: "@bob" }, enabled: true });
      } else if (name === "Rejected Last Admin") {
        click("button", "管理者を外す", { within: { text: "@bob" } });
        click("button", "管理者を外す", { within: { role: "alertdialog" } });
        wait("alert", "", { exact: false });
      }
    }
  }
  if (kind === "mfmsearch") {
    if (name === "Load Error") {
      wait("alert", "", { exact: false });
      wait("combobox", "検索エンジン", { enabled: false });
      wait("button", "再試行", { enabled: true });
    } else if (name === "Loading") wait("combobox", "検索エンジン", { enabled: false, busy: true });
    else {
      const initialEngine =
        name === "Duck Duck Go"
          ? "DuckDuckGo"
          : name === "Yandex" || name === "Baidu"
            ? name
            : "Google";
      wait("combobox", "検索エンジン", { enabled: true, value: initialEngine });
      wait("textbox", "検索語", {
        value: searchDemoQuery,
        attributes: { "aria-readonly": "true" },
        index: name === "Long Query Mobile" ? 1 : 0,
      });
      wait("button", `${initialEngine}で検索`, { index: name === "Long Query Mobile" ? 1 : 0 });
      if (["Change Engine", "Saving", "Save Error"].includes(name)) {
        select("検索エンジン", name === "Save Error" ? "Baidu" : "Yandex");
        if (name === "Save Error") {
          wait("alert", "", { exact: false });
          wait("combobox", "検索エンジン", { value: "Google", enabled: true });
        } else if (name === "Saving")
          wait("combobox", "検索エンジン", { enabled: false, busy: true, value: "Google" });
        else wait("combobox", "検索エンジン", { value: "Yandex" });
      }
    }
  }
  if (kind === "theme") {
    if (name === "Default") {
      wait("group", "テーマ");
      wait("button", "", { exact: false, count: 3, within: { role: "group", name: "テーマ" } });
      const active = typeof context.globals.theme === "string" ? context.globals.theme : "light";
      for (const [value, label] of [
        ["light", "ライト"],
        ["dark", "ダーク"],
        ["system", "システム"],
      ] as const)
        wait("button", label, { pressed: value === active });
    }
    if (name === "Dark") {
      click("button", "ダーク");
      wait("button", "ダーク", { pressed: true });
    }
    if (name === "Keyboard Selection") {
      click("button", "ライト");
      for (const key of ["ArrowRight", "ArrowRight", "Enter", "ArrowLeft", "Space"])
        interactions.push({ action: "key", key });
      wait("button", "ダーク", { pressed: true });
    }
    if (name === "Explanation") {
      interactions.push({ action: "hover", role: "button", name: "システム", exact: true });
      wait("tooltip", "", { exact: false });
    }
  }
  if (kind === "language") {
    if (name === "Open") {
      click("button", "言語を切り替える");
      wait("menuitemradio", "日本語");
    }
    if (name === "Select Control") {
      select("言語", "English");
      wait("combobox", "Language");
      select("Language", "日本語");
      wait("combobox", "言語");
    }
  }
  if (kind === "slackimport") {
    const exportName = "Zli Slack export Jan 1 2024 - Nov 1 2026.zip";
    if (name === "Empty") wait("button", "ZIP を選んでアップロード", { enabled: true });
    else if (name === "Load Error") wait("alert", "", { exact: false, text: "管理者だけ" });
    else if (name === "Mapping" || name === "Mobile") {
      // 取り込むチャンネルは管理者が選ぶので、初めはどれも取り込まない
      wait("button", "0 チャンネルを取り込む", { enabled: false });
      select("#general の取り込み方", "新しいチャンネル");
      wait("textbox", "#general の新しいチャンネル名", { valueExact: "slack-general" });
      wait("button", "1 チャンネルを取り込む", { enabled: true });
      // 以前取り込んだ #random は同じ取り込み先に固定する
      select("#random の取り込み方", "既存のチャンネル");
      wait("combobox", "#random の取り込み先", { enabled: false });
    } else if (name === "Started") {
      select("#general の取り込み方", "新しいチャンネル");
      click("button", "1 チャンネルを取り込む");
      wait("progressbar", "", { exact: false });
      wait("button", "0 チャンネルを取り込む", { enabled: false });
    } else if (name === "Start Error") {
      select("#general の取り込み方", "新しいチャンネル");
      click("button", "1 チャンネルを取り込む");
      wait("alert", "", { exact: false, text: "already taken" });
    } else if (name === "Failed Import") {
      wait("button", "履歴に移す", { enabled: true });
    } else if (name === "Importing")
      wait("progressbar", "", { exact: false, attributes: { "aria-valuenow": "33" } });
    else if (name === "Completed With Pending Files")
      wait("button", `${exportName} の添付ファイルを取得`, { enabled: true });
    else if (name === "History With Failed Files") click("button", "old.zip の添付ファイルを取得");
    else if (name === "Failed") {
      click("button", "ZIP を削除");
      wait("alertdialog", "", { exact: false });
    }
  }
  if (kind === "slacklink") {
    const dialog = { role: "alertdialog", name: "Slack との連携を解除しますか？" };
    if (name === "Not Linked") {
      click("button", "Slack と連携");
      interactions.push({
        action: "callback",
        name: "navigateTo",
        args: ["https://slack.com/oauth/v2/authorize?state=x"],
      });
    } else if (name === "Linked") {
      click("button", "連携を解除");
      wait("alertdialog", "", { exact: false });
    } else if (name === "Load Error") wait("alert", "", { exact: false });
    else if (name === "Not Configured") {
      // 設定がないときは連携ボタンを出さない
      wait("status", "", { exact: false, count: 0 });
      wait("button", "Slack と連携", { count: 0 });
    } else if (name === "Admin Manifest") wait("textbox", "Slack アプリの manifest");
    else if (name === "Returned With Error")
      wait("alert", "", { exact: false, text: "別の Zoo アカウントと連携済み" });
    else if (name === "Returned From Slack" || name === "Mobile")
      wait("button", "連携を解除", { enabled: true });
    else if (name === "Unlink Error") {
      click("button", "連携を解除");
      wait("alertdialog", "", { exact: false });
      click("button", "連携を解除", { within: dialog });
      // 背後のパネルはモーダルの裏になるので、ダイアログ内のエラーで確かめる
      wait("alert", "", { exact: false, text: "連携は残っています", within: dialog });
    } else if (name === "Returned From Slack Failed") {
      wait("alert", "", { exact: false, text: "有効期限" });
      wait("button", "Slack と連携", { enabled: true });
    } else if (name === "Own Import") {
      select("#design の取り込み方", "既存のチャンネル");
      wait("combobox", "#design の取り込み先", { value: "#design" });
      select("#secret の取り込み方", "新しいチャンネル");
      click("button", "2 チャンネルを取り込む");
      wait("progressbar", "", { exact: false, attributes: { "aria-valuenow": "33" } });
    } else if (name === "Own Import No Channels") {
      wait("button", "連携を解除", { enabled: true });
    }
  }
  if (kind === "pwa" && name === "Default")
    wait("button", "プッシュ通知", { exact: false, enabled: false });
  if (kind === "pwa" && name.includes("Install Available"))
    wait("button", "Zooをインストール", { enabled: true });
  if (kind === "pwa" && name === "Select Off") {
    wait("combobox", "プッシュ通知", { value: "無効", enabled: true });
    select("プッシュ通知", "有効");
    wait("combobox", "プッシュ通知", { value: "無効", enabled: true, busy: false });
  }
  const fixture = {
    ...base.fixture,
    kind,
    state,
    sharedApi: true,
    interactions,
    pushToggleCallback: pushState !== undefined,
    searchQuery: searchDemoQuery,
    longSearchQuery: name === "Long Query Mobile" ? longSearchDemoQuery : "",
    // Slack 取り込みは管理者だけが使う
    isAdmin:
      kind === "slackimport" ||
      (kind === "settings" && name === "Slack Import") ||
      (kind === "slacklink" && name === "Admin Manifest") ||
      [
        "Admin",
        "Mobile",
        "Protected Admin",
        "Last Admin",
        "Members",
        "Grant And Revoke",
        "Rejected Last Admin",
      ].includes(name),
    presentation: args.presentation ?? (name.startsWith("Select ") ? "select" : "icon"),
    capabilities: {
      installAvailable: name.includes("Install Available"),
      pushSupported: pushState?.supported ?? false,
      pushConfigured: Boolean(pushState?.publicKey),
      pushEnabled: pushState?.enabled ?? false,
      pushBusy: pushState?.busy ?? false,
      pushBlocked: pushState?.blocked ?? false,
      pushLabel: pushState?.label ?? "",
    },
  };
  return {
    ...base,
    sample: "preference-story",
    callbacks:
      kind === "pwa" ? { ...base.callbacks, pushToggle: pushStoryControls.toggle } : base.callbacks,
    surfaceClassName: preferenceSurfaces[title as keyof typeof preferenceSurfaces],
    fullHeight: title === "Pages/ZooSettings",
    path:
      title === "Pages/ZooSettings"
        ? "/settings?tab=general&channel=channel-general"
        : kind === "slacklink"
          ? // Slack からの戻り (?code=&state= など) を Story と同じ URL で始める
            (parameters.router?.initialEntries?.[0] ?? "/settings?tab=slack")
          : base.path,
    fixture,
  };
}
