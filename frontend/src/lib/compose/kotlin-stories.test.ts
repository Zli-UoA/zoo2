import { describe, expect, it } from "vite-plus/test";
import { kotlinOptions, type Selection } from "../../../.storybook/kotlin-stories";

function story(
  title: string,
  name: string,
  args: Record<string, unknown> = {},
  parameters: Record<string, unknown> = {},
): Selection {
  return {
    id: "story--state",
    title,
    name,
    args,
    parameters,
    globals: { implementation: "kotlin", theme: "dark", locale: "en" },
  };
}

describe("existing Storybook stories rendered in Kotlin", () => {
  it("shares reaction actors and labels from the existing reaction-user story", () => {
    const options = kotlinOptions(
      story("Features/Chat/MessageRow", "Reaction Users", {
        item: { message: { id: "m", user_id: "author" }, isGroupStart: true },
        reactions: [
          {
            emoji: "👍",
            userIds: ["bob", "long"],
            userNames: { bob: "Bob (@bob)", long: `${"A".repeat(80)} (@longname)` },
          },
        ],
      }),
    )!;
    expect(options.fixture?.reactions).toEqual([
      { id: "reaction-0-bob", message_id: "m", user_id: "bob", emoji: "👍" },
      { id: "reaction-0-long", message_id: "m", user_id: "long", emoji: "👍" },
    ]);
    expect(options.fixture?.users).toContainEqual({
      id: "long",
      display_name: "A".repeat(80),
      user_name: "longname",
    });
  });
  it("uses the existing message and attachment arguments without mutating them", () => {
    const message = {
      id: "message-1",
      user_id: "author",
      content: "Same Controls",
      display_name: "Alice",
      user_name: "alice",
    };
    const attachment = { id: "file-1", message_id: "message-1" };
    const input = story("Features/Chat/MessageRow", "With Attachments", {
      item: { message, isGroupStart: false },
      attachments: [attachment],
    });
    const options = kotlinOptions(input)!;
    expect(options.content).toBe(message.content);
    expect(options.grouped).toBe(true);
    expect(options.fixture?.attachments).toEqual([attachment]);
    expect(input.args.item).toEqual({ message, isGroupStart: false });
    expect(options.locale).toBe("en");
    expect(options.dark).toBe(true);
  });

  it("updates the disabled Composer through its existing Controls", () => {
    const input = story("Features/Chat/Composer", "Default", { disabled: false });
    expect(kotlinOptions(input)?.disabled).toBe(false);
    input.args.disabled = true;
    expect(kotlinOptions(input)?.disabled).toBe(true);
  });

  it("preserves an explicitly empty shared fixture instead of substituting demo data", () => {
    const options = kotlinOptions(
      story("Pages/Notifications", "Empty", {}, { collections: { notifications: [], users: [] } }),
    );
    expect(options?.fixture?.notifications).toEqual([]);
    expect(options?.fixture?.users).toEqual([]);
  });

  it("passes the current card and App blocks rather than unrelated sample content", () => {
    const card = { title: "Shared deployment", fields: [{ title: "status", value: "done" }] };
    expect(
      kotlinOptions(story("Features/Chat/MessageCard", "Deployment", { card }))?.fixture?.card,
    ).toBe(card);
    const blocks = [{ type: "card", card }];
    expect(kotlinOptions(story("Features/Apps/Blocks", "Card", { blocks }))?.fixture?.blocks).toBe(
      blocks,
    );
  });

  it("does not claim unregistered components or unsupported states are Kotlin previews", () => {
    expect(kotlinOptions(story("Components/Button", "Default"))).toBeNull();
    expect(kotlinOptions(story("Features/Chat/MessageRow", "Editing"))).toBeNull();
  });
});
