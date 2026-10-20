import { describe, expect, it } from "vite-plus/test";

import { countRepliesByParent } from "./threads";

describe("countRepliesByParent", () => {
  it("counts replies per root and ignores top-level messages", () => {
    const counts = countRepliesByParent([
      { parent_id: "root-1" },
      { parent_id: "root-1" },
      { parent_id: "root-2" },
      { parent_id: null },
    ]);

    expect(counts.get("root-1")).toBe(2);
    expect(counts.get("root-2")).toBe(1);
    expect(counts.size).toBe(2);
  });

  it("returns an empty map when there are no replies", () => {
    expect(countRepliesByParent([{ parent_id: null }]).size).toBe(0);
  });
});
