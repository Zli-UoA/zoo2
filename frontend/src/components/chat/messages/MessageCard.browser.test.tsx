import { render } from "@testing-library/react";
import { expect, it } from "vite-plus/test";
import { MessageCard } from "./MessageCard";
import "../../../styles.css";

it.each([
  [300, 1],
  [700, 2],
])("lays out card fields within a %ipx container", (width, columns) => {
  const view = render(
    <div style={{ width }}>
      <MessageCard
        attachment
        card={{
          title: "Deployment",
          description: "",
          url: "https://example.com/deploy",
          source: "GitHub",
          context: "",
          author: "",
          badge: "",
          fields: [
            { title: "Status", value: "Completed", short: true },
            { title: "Commit", value: "a-long-commit-description", short: true, code: true },
            { title: "Workflow", value: "Release" },
          ],
        }}
      />
    </div>,
  );
  const fields = view.container.querySelector("dl")!;
  expect(getComputedStyle(fields).gridTemplateColumns.split(" ")).toHaveLength(columns);
  expect(view.container.firstElementChild!.scrollWidth).toBeLessThanOrEqual(width);
});
