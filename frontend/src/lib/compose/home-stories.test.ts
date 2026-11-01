import { describe, expect, it } from "vite-plus/test";
import source from "../../components/home/HomeScreen.stories.tsx?raw";
import { homeInteractions, homeOptions } from "../../../.storybook/home-stories";
import type { Selection } from "../../../.storybook/kotlin-stories";
import type { KotlinPreviewProps } from "../../../.storybook/KotlinPreview";
const base: KotlinPreviewProps = {
  sample: "message",
  content: "",
  dark: false,
  locale: "ja",
  grouped: false,
  disabled: false,
  fixture: { sharedApi: true },
};
const exports = [...source.matchAll(/export const (\w+): Story/g)].map((match) => match[1]);
const kebab = (name: string) => name.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
describe("Home uses the existing stories and their real API", () => {
  it.each(exports)("registers %s with serialized real interaction descriptors", (name) => {
    const context: Selection = {
      id: `pages-home--${kebab(name)}`,
      title: "Pages/Home",
      name,
      args: {},
      parameters: {},
      globals: {},
    };
    const options = homeOptions(context, base)!;
    expect(options.sample).toBe("home-story");
    expect(options.fixture?.sharedApi).toBe(true);
    expect(options.fixture?.interactions).toEqual(homeInteractions[kebab(name)]);
    expect(options.fixture?.interactions).not.toHaveLength(0);
    expect(() => structuredClone(options)).not.toThrow();
    expect(homeOptions({ ...context, id: `Pages/Home--${name}` }, base)).toEqual(options);
  });
  it("maps exactly the exports without a fallback that hides missing states", () => {
    expect(Object.keys(homeInteractions).sort()).toEqual(exports.map(kebab).sort());
  });
  it("retains server acknowledgement and websocket race assertions", () => {
    expect(homeInteractions.submit.at(-1)).toMatchObject({ action: "wait", text: "公開する近況" });
    expect(
      homeInteractions["live-while-loading-older"]
        .filter((step) => step.action === "fixture")
        .map((step) => step.name),
    ).toEqual(["homeOlderLiveReady", "homeOlderRefresh", "homeOlderRetained"]);
  });
});
