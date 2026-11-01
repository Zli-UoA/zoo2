import { storyDeploymentCard } from "./app-fixtures";
import type { Selection } from "./kotlin-stories";
import type { KotlinPreviewProps } from "./KotlinPreview";

export function appOptions(
  context: Selection,
  base: KotlinPreviewProps,
): KotlinPreviewProps | null {
  const { title, name, args, parameters } = context;
  const surfaces: Record<string, string> = {
    "Features/Apps/Blocks": "mx-auto max-w-md p-4",
    "Flows/Apps/SubmitAndRetry": "mx-auto max-w-md p-4",
    "Features/Apps/Connections": "mx-auto max-w-xl p-4",
    "Features/Apps/Catalog": "mx-auto h-screen max-w-5xl p-4",
  };
  if (
    !(title in surfaces) &&
    title !== "Pages/AppConsent" &&
    title !== "Features/Channels/Webhooks"
  )
    return null;
  const query = new URLSearchParams({
    app_id: "consent-story",
    scope: "messages:read messages:write",
    redirect_uri: "https://example.com/callback",
    code_challenge_method: "S256",
    state: "storybook-state",
    code_challenge: "storybook-challenge",
  });
  const interactions: Record<string, unknown>[] = [];
  const click = (role: string, label: string, exact = true) =>
    interactions.push({ action: "click", role, name: label, exact });
  const wait = (role: string, label: string, state = "visible", exact = true) =>
    interactions.push({ action: "wait", role, name: label, state, exact });
  if (
    (title === "Features/Apps/Blocks" && ["Pending", "Failed"].includes(name)) ||
    title === "Flows/Apps/SubmitAndRetry"
  ) {
    interactions.push({
      action: "type",
      role: "textbox",
      name: "通知のタイトル",
      value: "リリース完了",
      exact: true,
    });
    if (title.startsWith("Flows/"))
      interactions.push({ action: "select", role: "combobox", name: "優先度", value: "高" });
    click("button", "送信");
    if (name === "Pending")
      interactions.push({ action: "wait", role: "button", name: "送信", enabled: false });
    else {
      interactions.push({ action: "wait", role: "alert", state: "visible" });
      interactions.push({
        action: "wait",
        role: "textbox",
        name: "通知のタイトル",
        valueExact: "リリース完了",
      });
      if (title.startsWith("Flows/"))
        interactions.push({ action: "wait", role: "combobox", name: "優先度", valueExact: "高" });
    }
    if (title.startsWith("Flows/")) {
      click("button", "送信");
      interactions.push({ action: "wait", role: "alert", state: "hidden" });
    }
  }
  if (title === "Features/Apps/Blocks" && name === "Card") {
    interactions.push({
      action: "wait",
      role: "link",
      name: storyDeploymentCard.title,
      attributes: { href: storyDeploymentCard.url },
    });
    const commit = storyDeploymentCard.fields?.find((field) => field.title === "Commit");
    interactions.push({
      action: "wait",
      role: "link",
      name: commit?.value,
      attributes: { href: commit?.url },
    });
  }
  if (title === "Pages/AppConsent" && ["Default", "Mobile"].includes(name)) {
    wait("heading", "Release Tracker");
    interactions.push({ action: "wait", role: "checkbox", count: 2 });
    interactions.push({
      action: "wait",
      role: "complementary",
      name: "チャンネル",
      state: "hidden",
    });
  }
  if (title === "Features/Channels/Webhooks") {
    if (name === "Default") {
      // Compose also projects the editable name's value as text, unlike <input>.
      interactions.push({ action: "wait", text: "Webhook", exact: true, count: 2 });
      wait("button", "Webhook URLを表示");
    }
    if (name === "Empty")
      interactions.push({ action: "wait", text: "Webhookはまだありません。", exact: true });
  }
  if (title === "Features/Apps/Catalog") {
    if (name === "Search Empty") {
      wait("button", "Echo Bot", "visible", false);
      interactions.push({
        action: "type",
        role: "textbox",
        name: "Appを検索",
        value: "nonexistent-app",
        exact: true,
      });
      interactions.push({
        action: "wait",
        role: "button",
        name: "Echo Bot",
        exact: false,
        state: "hidden",
      });
    }
    if (["Details", "Owner Management", "Mobile", "Other Owner"].includes(name)) {
      click("button", name === "Other Owner" ? "Meeting Notes" : "Echo Bot", false);
      if (name === "Owner Management") {
        click("tab", "管理・開発");
        wait("button", "トークンを発行");
      } else if (name === "Other Owner") {
        wait("tab", "使う");
        wait("tab", "管理・開発", "hidden");
      } else wait("tab", "管理・開発");
    }
  }
  if (
    (name === "Load Error" && ["Features/Apps/Connections", "Pages/AppConsent"].includes(title)) ||
    (title === "Pages/AppConsent" && name === "Invalid Request")
  )
    interactions.push({ action: "wait", role: "alert", state: "visible" });
  return {
    ...base,
    sample: "app-story" as KotlinPreviewProps["sample"],
    surfaceClassName: surfaces[title],
    fullHeight:
      title === "Pages/AppConsent" ||
      title === "Features/Channels/Webhooks" ||
      title === "Features/Apps/Catalog",
    path:
      title === "Pages/AppConsent"
        ? `/apps-authorize?${query}`
        : parameters.router?.initialEntries?.[0],
    fixture: {
      ...base.fixture,
      storyTitle: title,
      state: name,
      interactions,
      callbacks: Object.keys(args).filter((key) => typeof args[key] === "function"),
      appId: args.appId ?? "echo",
      blocks: args.blocks,
      surfaceId: args.surfaceId,
      messageId: args.messageId,
      channelId:
        args.channelId ?? (title === "Features/Apps/Connections" ? "channel-general" : null),
      submit: Boolean(args.submit),
      connections: Boolean(args.connections),
      assistant: Boolean(args.assistant),
      channel: args.channel,
      initialApp: null,
      initialSearch: "",
      initialManagement: false,
    },
  };
}
