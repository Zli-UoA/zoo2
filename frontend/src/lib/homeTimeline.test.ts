import { describe, expect, it } from "vite-plus/test";
import { reachedTimelineTail, timelinePosition } from "./homeTimeline";
describe("retaining the loaded timeline tail", () => {
  it("does not stop at a newer microsecond in the same millisecond", () => {
    const tail = timelinePosition({ id: "z-tail", sort_key: "1767225600000001" });
    expect(reachedTimelineTail([{ id: "a-newer", sort_key: "1767225600000002" }], tail)).toBe(
      false,
    );
    expect(reachedTimelineTail([{ id: "z-tail", sort_key: "1767225600000001" }], tail)).toBe(true);
  });
  it("does not use ID collation to stop within an equal timestamp", () => {
    const tail = timelinePosition({ id: "z-tail", sort_key: "1767225600000001" });
    expect(reachedTimelineTail([{ id: "a-same-time", sort_key: "1767225600000001" }], tail)).toBe(
      false,
    );
    expect(
      reachedTimelineTail([{ id: "past-deleted-tail", sort_key: "1767225600000000" }], tail),
    ).toBe(true);
  });
  it("keeps exact ordering beyond Number precision and for repost entry IDs", () => {
    const tail = timelinePosition({
      id: "original",
      entry_id: "repost:tail",
      sort_key: "9007199254740993",
    });
    expect(
      reachedTimelineTail(
        [{ id: "original", entry_id: "repost:other", sort_key: "9007199254740994" }],
        tail,
      ),
    ).toBe(false);
    expect(
      reachedTimelineTail(
        [{ id: "original", entry_id: "repost:tail", sort_key: "9007199254740993" }],
        tail,
      ),
    ).toBe(true);
  });
});
