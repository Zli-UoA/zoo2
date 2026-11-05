import { storySurfaces } from "../../../../.storybook/story-surfaces";
import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, within } from "storybook/test";
import { LinkPreviews } from "./LinkPreviews.tsx";
const svg = (wide: boolean) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${wide ? 1200 : 400}" height="630"><rect width="100%" height="100%" fill="#2468aa"/></svg>`;
const meta = {
  title: "Features/Chat/LinkPreviews",
  component: LinkPreviews,
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/summaly-endpoint", () =>
          HttpResponse.text("https://storybook.example/summaly"),
        ),
        http.get("*/media-proxy-endpoint", () =>
          HttpResponse.text("https://storybook.example/media"),
        ),
        http.get("https://storybook.example/summaly", ({ request }) => {
          const kind = new URL(new URL(request.url).searchParams.get("url")!).pathname;
          return HttpResponse.json({
            title: "Zooのリリース情報",
            description: "プレビューの横幅と本文の折り返しを確認します。".repeat(6),
            sitename: "Zoo Blog",
            thumbnail:
              kind === "/text"
                ? null
                : `https://storybook.example/${kind.includes("portrait") ? "portrait" : "wide"}.svg`,
          });
        }),
        http.get("https://storybook.example/media", ({ request }) =>
          HttpResponse.text(
            svg(!new URL(request.url).searchParams.get("url")?.includes("portrait")),
            { headers: { "Content-Type": "image/svg+xml" } },
          ),
        ),
      ],
    },
  },
  decorators: [
    (Story) => (
      <div className={storySurfaces["Features/Chat/LinkPreviews"]}>
        <Story />
      </div>
    ),
  ],
  args: { content: "https://example.com/wide" },
} satisfies Meta<typeof LinkPreviews>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Landscape: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    const card = await within(canvasElement).findByRole("link", { name: /Zooのリリース情報/ });
    await expect(card.getBoundingClientRect().width).toBeLessThanOrEqual(384);
  }),
};
export const Portrait: Story = { args: { content: "https://example.com/portrait" } };
export const TextOnly: Story = { args: { content: "https://example.com/text" } };
export const Mobile: Story = { ...Landscape, globals: { viewport: { value: "mobile" } } };
