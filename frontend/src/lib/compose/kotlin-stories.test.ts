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
      { id: expect.any(String), message_id: "m", user_id: "bob", emoji: "👍" },
      { id: expect.any(String), message_id: "m", user_id: "long", emoji: "👍" },
    ]);
    const reactions = options.fixture?.reactions as { id: string }[];
    expect(new Set(reactions.map(({ id }) => id)).size).toBe(reactions.length);
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
    const args = options.fixture?.args as Record<string, unknown>;
    expect(args.item).toEqual({ message, isGroupStart: false });
    expect(args.attachments).toEqual([attachment]);
    expect(input.args.item).toEqual({ message, isGroupStart: false });
    expect(options.locale).toBe("en");
    expect(options.dark).toBe(true);
  });

  it("retains the shared MessagePane reactions and explicit membership Control", () => {
    const reactions = [{ id: "r", message_id: "m", user_id: "alice", emoji: "👍" }];
    const options = kotlinOptions(
      story(
        "Features/Chat/MessagePane",
        "Not Joined",
        {
          channelId: "channel-general",
          isMember: false,
        },
        { collections: { reactions } },
      ),
    )!;
    expect(options.fixture?.reactions).toEqual(reactions);
    expect(options.fixture?.args).toMatchObject({ isMember: false });
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
      (
        kotlinOptions(story("Features/Chat/MessageCard", "Deployment", { card }))?.fixture
          ?.args as Record<string, unknown>
      )?.card,
    ).toEqual(card);
    const blocks = [{ type: "card", card }];
    expect(kotlinOptions(story("Features/Apps/Blocks", "Card", { blocks }))?.fixture?.blocks).toBe(
      blocks,
    );
  });

  it("rejects unknown components instead of substituting another Kotlin component", () => {
    expect(kotlinOptions(story("Components/Unknown", "Default"))).toBeNull();
    expect(kotlinOptions(story("Components/Button", "Default"))?.sample).toBe("button");
  });
});
