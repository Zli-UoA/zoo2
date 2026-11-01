import { describe, expect, it } from "vite-plus/test";
import { notificationOptions } from "../../../.storybook/notification-stories";
import type { KotlinPreviewProps } from "../../../.storybook/KotlinPreview";
import type { NativeInteraction } from "../../../.storybook/native-interactions";
import type { Selection } from "../../../.storybook/kotlin-stories";

const base: KotlinPreviewProps = {
  sample: "message",
  content: "",
  disabled: false,
  grouped: false,
  dark: false,
  locale: "ja",
  fixture: { currentUserId: "user-alice" },
};
function selection(
  title: string,
  name: string,
  args: Record<string, unknown> = {},
  parameters: Record<string, unknown> = {},
): Selection {
  return { id: "story", title, name, args, parameters, globals: {} } as Selection;
}

describe("Kotlin notification story input", () => {
  it("retains explicit missing DM metadata and the same grouped notifications", () => {
    const rows = [
      { id: "one", actor_id: "bob" },
      { id: "two", actor_id: "carol" },
    ];
    const channel = { id: "dm", is_dm: true };
    const options = notificationOptions(
      selection("Features/Notifications/Row", "Dm Metadata Pending", {
        group: { notifications: rows },
        users: new Map(),
        channel,
        readEnabled: false,
      }),
      base,
    )!;
    expect(options.fixture?.users).toEqual([]);
    expect(options.fixture?.notifications).toBe(rows);
    expect(options.fixture?.channel).toBe(channel);
    expect(options.fixture?.readEnabled).toBe(false);
  });
  it("uses the same effective local collection config as the React SyncStatus", () => {
    const options = notificationOptions(
      selection(
        "Features/Status/Sync",
        "Direct",
        {},
        {
          collections: { config: { shapeMode: "direct", txidMode: "awaitMatch", devUserId: null } },
        },
      ),
      base,
    )!;
    expect(options.fixture?.config).toMatchObject({
      shapeMode: "direct",
      txidMode: "skip",
      devUserId: "user-alice",
    });
  });
  it.each(["Unread", "Single Unread", "All Read"])(
    "registers the existing notification button %s",
    (name) => {
      const notifications = [{ id: "one", user_id: "user-alice", is_read: name === "All Read" }];
      const options = notificationOptions(selection("Features/Notifications/Button", name), {
        ...base,
        fixture: { ...base.fixture, notifications },
      })!;
      expect(options.sample).toBe("notification-story");
      expect(options.fixture?.view).toBe("button");
      expect(options.fixture?.notifications).toBe(notifications);
      expect(options.measureWidth).toBe(true);
    },
  );
  it("retains the OfflineContext retry function as an actual callback", () => {
    const retryPersistence = () => Promise.resolve();
    const options = notificationOptions(
      selection(
        "Features/Status/Offline",
        "Persistence Error",
        {},
        {
          offline: { isOnline: true, persistenceStatus: "error", retryPersistence },
        },
      ),
      base,
    )!;
    expect(options.callbacks?.retryPersistence).toBe(retryPersistence);
    expect(options.fixture?.callbacks).toContain("retryPersistence");
  });
  it("matches only the stories that explicitly disable focus-driven read marking", () => {
    expect(
      notificationOptions(selection("Pages/Notifications", "Default"), base)?.fixture?.focused,
    ).toBe(false);
    expect(
      notificationOptions(selection("Pages/Notifications", "Single Unread"), base)?.fixture
        ?.focused,
    ).toBeUndefined();
  });
});

describe("native notification plays", () => {
  function interactions(title: string, name: string): NativeInteraction[] {
    return notificationOptions(selection(title, name), base)!.fixture!
      .interactions as NativeInteraction[];
  }
  it.each([
    ...[
      "Default",
      "Single Unread",
      "Empty",
      "Mobile",
      "Dark",
      "Direct Messages",
      "Message Layout Comparison",
      "Message Layout Comparison Mobile",
      "Loading",
      "Bodies Ready Together",
      "Loading On Scroll",
    ].map((name) => ["Pages/Notifications", name]),
    ...[
      "Mention",
      "Direct Message",
      "Group Direct Message",
      "Dm Metadata Pending",
      "Grouped Reactions",
      "Loading",
      "Unavailable",
      "Long Content Mobile",
      "Native Interactions",
    ].map((name) => ["Features/Notifications/Row", name]),
  ])("keeps actual assertions for %s / %s", (title, name) => {
    expect(interactions(title!, name!).some((step) => step.action === "wait")).toBe(true);
  });
  it("exercises loading more and returning to already loaded content", () => {
    const steps = interactions("Pages/Notifications", "Loading On Scroll");
    expect(
      steps.filter((step) => step.action === "scroll").map((step) => Math.sign(step.deltaY!)),
    ).toEqual([1, -1]);
    const down = steps.findIndex((step) => step.action === "scroll");
    const up = steps.map((step) => step.action).lastIndexOf("scroll");
    expect(
      steps.slice(down, up).some((step) => step.role === "status" && step.state !== "hidden"),
    ).toBe(true);
    expect(steps.slice(up).some((step) => step.text === "本文 scroll-0")).toBe(true);
  });
  it("releases real response bodies only after observing initial loading", () => {
    const steps = interactions("Pages/Notifications", "Bodies Ready Together");
    const release = steps.findIndex((step) => step.action === "fixture");
    expect(release).toBeGreaterThan(0);
    expect(
      steps.slice(0, release).some((step) => step.role === "status" && step.state !== "hidden"),
    ).toBe(true);
    expect(steps.slice(release).some((step) => step.count === 4)).toBe(true);
  });
  it("verifies selection suppression, ordinary navigation and modifier navigation", () => {
    const steps = interactions("Features/Notifications/Row", "Native Interactions");
    expect(steps.filter((step) => step.action === "pointer")).toHaveLength(3);
    expect(steps.filter((step) => step.action === "navigation").map((step) => step.count)).toEqual([
      0, 1, 2,
    ]);
    expect(steps.some((step) => step.action === "pointer" && step.ctrlKey)).toBe(true);
  });
});
