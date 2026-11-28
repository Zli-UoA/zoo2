// Shared by the locale tests and the lightweight CI validation.
const languageIndependentKeys = new Set([
  "onboarding.progress", // The step counter is an interpolation-only numeric layout.
  "catchUp.progress", // Page positions are an interpolation-only numeric layout.
  "channels.descriptionCounter",
  "auth.errorWithCode",
  "reactions.chip",
  "reactions.kitchen",
  "bubbleChat.title",
  "bubbleChat.navTitle",
  "home.title", // The navigation name is explicitly Home in Japanese too.
  "reactions.tabKitchen",
  "webhooks.url",
  "dms.moreNames",
  "appConsole.events.websocket", // Protocol and Slack field names stay untranslated.
  "appConsole.events.requestUrl",
  "messages.typingSeparator", // Korean also separates names with a comma and a space.
  "e2ee.deviceName.withOs", // Korean also writes the OS in half-width parentheses.
  // Android's revocation notes: Korean uses the same half-width colon, comma and parentheses as English.
  "android.encryption.revocation.noteLine",
  "android.encryption.revocation.reasonSeparator",
  "android.encryption.revocation.deviceLine",
]);

export function englishCopyKeys(
  english: Record<string, string>,
  target: Record<string, string>,
): string[] {
  return Object.entries(target)
    .filter(([key, value]) => value === english[key] && !languageIndependentKeys.has(key))
    .map(([key]) => key);
}

export function translationVariables(value: string): string[] | null {
  // Android substitutes the exact {{name}} token in shared dictionaries.
  const interpolation = /{{(\w+)}}/g;
  if (/{{|}}/.test(value.replace(interpolation, ""))) return null;
  return [...new Set([...value.matchAll(interpolation)].map(([, name]) => name!))].sort();
}
