import { blobcat, misskeyPost } from "./activitypub-fixtures";
import { queryAllByRole } from "@testing-library/react";
import { expect, waitFor } from "storybook/test";

function frame() {
  const result = document.querySelector<HTMLIFrameElement>(
    'iframe[title="Kotlin component preview"]',
  );
  if (!result?.contentDocument?.defaultView) throw new Error("Home Kotlin iframe is unavailable");
  return result;
}
function roots() {
  const result: (Document | ShadowRoot)[] = [frame().contentDocument!];
  for (let index = 0; index < result.length; index++)
    for (const item of result[index].querySelectorAll("*"))
      if (item.shadowRoot) result.push(item.shadowRoot);
  return result;
}
function elements(role: string, name?: string) {
  return roots().flatMap((root) =>
    queryAllByRole(root as unknown as HTMLElement, role, name === undefined ? {} : { name }),
  );
}
function element(role: string, name?: string) {
  const values = elements(role, name);
  if (values.length !== 1)
    throw new Error(`Home expected one ${role} ${name ?? ""}; found ${values.length}`);
  return values[0];
}
function activeEditor() {
  let current = frame().contentDocument!.activeElement;
  while (current?.shadowRoot?.activeElement) current = current.shadowRoot.activeElement;
  return current?.tagName === "INPUT" || current?.tagName === "TEXTAREA" ? current : null;
}
let previousComposer: HTMLElement | undefined;
let previousComposerId: string | undefined;
let finishPull: (() => void) | undefined;
let focusedDuringClick = false;
let stopFocusObservation: (() => void) | undefined;
export const nativeHomeChecks = {
  homeBeforeRemount() {
    previousComposer = element("button", "投稿を書く");
    previousComposerId = previousComposer.id;
  },
  async homeAfterRemount() {
    await waitFor(async () => {
      const current = element("button", "投稿を書く");
      await expect(current !== previousComposer || current.id !== previousComposerId).toBe(true);
      await expect(current.getAttribute("aria-disabled")).toBe("false");
    });
  },
  homeObserveComposerFocus() {
    stopFocusObservation?.();
    focusedDuringClick = false;
    const document = frame().contentDocument!;
    const listener = () => {
      focusedDuringClick = activeEditor() !== null;
    };
    document.addEventListener("click", listener);
    stopFocusObservation = () => document.removeEventListener("click", listener);
  },
  async homeComposerFocused() {
    stopFocusObservation?.();
    stopFocusObservation = undefined;
    await expect(focusedDuringClick).toBe(true);
    await expect(activeEditor()).not.toBeNull();
  },
  async homeToolbar() {
    const toolbar = element("group", "投稿設定");
    const buttons = queryAllByRole(toolbar, "button");
    await expect(buttons.map((button) => button.getAttribute("aria-label"))).toEqual([
      "公開範囲: ホーム",
      "連合あり",
      "投稿",
    ]);
    await expect(buttons.every((button) => !button.textContent?.trim())).toBe(true);
  },
  async homeVisibilityIcons() {
    const items = elements("menuitemradio");
    await expect(items.map((item) => item.getAttribute("aria-label") ?? item.textContent)).toEqual([
      "パブリック",
      "ホーム",
      "フォロワー限定",
    ]);
    // Icons are painted into Canvas; the paired screenshot verifies their presence.
    // Role, order and selection are tested independently of decorative SVG DOM.
  },
  async homeActionBar() {
    const group = element("group", "投稿のアクション");
    await expect(
      queryAllByRole(group, "button").map((button) => button.getAttribute("aria-label")),
    ).toEqual(["返信 0", "リポスト 0", "リアクション 0", "投稿の操作"]);
    await expect(queryAllByRole(group, "time")).toHaveLength(0);
  },
  async homeAvatarLinks() {
    const article = element("article");
    const links = queryAllByRole(article, "link");
    await expect(links.some((link) => link.getAttribute("href")?.includes("/home/users/"))).toBe(
      true,
    );
    const postLinks = links.filter((link) => link.getAttribute("href")?.includes("/home/posts/"));
    await expect(postLinks.length).toBeGreaterThanOrEqual(1);
    const times = queryAllByRole(article, "time");
    await expect(times.length).toBeGreaterThanOrEqual(1);
    await expect(
      times.every((time) =>
        postLinks.some((link) => {
          const outer = link.getBoundingClientRect(),
            inner = time.getBoundingClientRect();
          return (
            outer.left <= inner.left &&
            outer.right >= inner.right &&
            outer.top <= inner.top &&
            outer.bottom >= inner.bottom
          );
        }),
      ),
    ).toBe(true);
  },
  async homeChannelMetadata() {
    const link = element("link", "#general");
    const times = elements("time");
    const box = link.getBoundingClientRect();
    await expect(
      times.some((time) => {
        const bounds = time.getBoundingClientRect();
        return (
          bounds.right <= box.left &&
          Math.abs(bounds.top - box.top) < Math.max(bounds.height, box.height)
        );
      }),
    ).toBe(true);
  },
  async homeComposerDesktop() {
    const bounds = element("dialog", "近況を投稿").getBoundingClientRect();
    await expect(bounds.left).toBeGreaterThan(0);
    await expect(bounds.width).toBeLessThan(frame().contentWindow!.innerWidth);
  },
  async homeComposerMobile() {
    await nativeHomeChecks.homeComposerFocused();
    const child = frame().contentWindow!;
    const check = async (height: number) => {
      const bounds = element("dialog", "近況を投稿").getBoundingClientRect();
      await expect(Math.abs(bounds.left)).toBeLessThan(1);
      await expect(Math.abs(bounds.top)).toBeLessThan(1);
      await expect(Math.abs(bounds.width - child.innerWidth)).toBeLessThan(1);
      await expect(Math.abs(bounds.height - height)).toBeLessThan(1);
    };
    await waitFor(() => check(child.innerHeight));
    const height = child.innerHeight - 300;
    const viewports = [window.visualViewport!, child.visualViewport!];
    const originals = viewports.map((viewport) =>
      Object.getOwnPropertyDescriptor(viewport, "height"),
    );
    try {
      viewports.forEach((viewport) => {
        Object.defineProperty(viewport, "height", { configurable: true, value: height });
        viewport.dispatchEvent(new Event("resize"));
      });
      await waitFor(() => check(height));
    } finally {
      viewports.forEach((viewport, index) => {
        if (originals[index]) Object.defineProperty(viewport, "height", originals[index]!);
        else Reflect.deleteProperty(viewport, "height");
        viewport.dispatchEvent(new Event("resize"));
      });
    }
  },
  async homePullStart() {
    const feed = element("feed");
    const bounds = feed.getBoundingClientRect();
    const owner = frame().contentDocument!.defaultView as Window & typeof globalThis;
    const canvas = roots()
      .flatMap((root) => [...root.querySelectorAll("canvas")])
      .filter((canvas) => canvas.getBoundingClientRect().width > 0);
    if (canvas.length !== 1) throw new Error("Home pull expected one Canvas");
    const x = bounds.left + bounds.width / 2;
    const y = bounds.top + 20;
    const send = (type: string, clientY: number, buttons: number) =>
      canvas[0].dispatchEvent(
        new owner.PointerEvent(type, {
          pointerId: 321,
          pointerType: "touch",
          isPrimary: true,
          clientX: x,
          clientY,
          button: 0,
          buttons,
          bubbles: true,
          cancelable: true,
          composed: true,
        }),
      );
    send("pointerdown", y, 1);
    await new Promise<void>((resolve) => owner.requestAnimationFrame(() => resolve()));
    send("pointermove", y + 160, 1);
    finishPull = () => {
      send("pointerup", y + 160, 0);
    };
  },
  homePullFinish() {
    if (!finishPull) throw new Error("Home pull gesture has not started");
    finishPull();
    finishPull = undefined;
  },
  async homeEmojiFallback() {
    await waitFor(async () => {
      const leaves = roots()
        .flatMap((root) => [...root.querySelectorAll("*")])
        .filter((node) => !node.children.length);
      const occurrences = leaves.reduce(
        (count, node) => count + ((node.textContent ?? "").split(blobcat.name).length - 1),
        0,
      );
      await expect(occurrences).toBeGreaterThanOrEqual(2);
      await expect(elements("img", ":blobcat:")).toHaveLength(0);
    });
  },
  async homeEmojiImages() {
    await waitFor(async () => {
      // Author/body expose decoded images; the chip merges its image into its button.
      // The paired Canvas screenshot verifies all three painted images.
      const text = roots()
        .flatMap((root) => [...root.querySelectorAll("*")])
        .map((node) => node.textContent ?? "");
      await expect(elements("img", blobcat.name)).toHaveLength(2);
      await expect(text).toContain(misskeyPost.display_name);
      await expect(text).toContain(misskeyPost.content);
      await expect(text.join("")).toContain(`@${misskeyPost.user_name}`);
      const chips = elements("button").filter(
        (button) => button.getAttribute("aria-label")?.replace(/\s/g, "") === `${blobcat.name}2`,
      );
      await expect(chips).toHaveLength(1);
    });
  },
};
