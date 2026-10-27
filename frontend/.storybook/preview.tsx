import type { Preview } from "@storybook/react-vite";

import "../src/styles.css";
import { startStorybookWorker, withCollections, withMsw, withProviders } from "./decorators";

// アプリの Service Worker が MSW の worker を上書きしないよう、Storybook では
// PWA の登録処理を無効にする。
if (typeof window !== "undefined") {
  try {
    Object.defineProperty(window, "isSecureContext", { value: false, configurable: true });
  } catch {
    // 読み取り専用の環境では PWA の登録だけが無効にならない。
  }
}

const preview: Preview = {
  beforeAll: async () => {
    await startStorybookWorker();
  },
  parameters: {
    layout: "fullscreen",
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    a11y: { test: "todo" },
    backgrounds: {
      options: {
        app: { name: "app", value: "var(--background)" },
        muted: { name: "muted", value: "var(--muted)" },
      },
    },
    viewport: {
      options: {
        desktop: {
          name: "Desktop",
          styles: { width: "1440px", height: "900px" },
          type: "desktop",
        },
        mobile: {
          name: "Mobile",
          styles: { width: "390px", height: "844px" },
          type: "mobile",
        },
      },
    },
    options: {
      storySort: {
        order: ["Chat", "Notifications", "Layout", "Foundations", "Screens"],
      },
    },
  },
  globalTypes: {
    theme: {
      description: "Theme",
      toolbar: {
        icon: "circlehollow",
        items: [
          { value: "light", title: "Light" },
          { value: "dark", title: "Dark" },
        ],
        dynamicTitle: true,
      },
    },
    locale: {
      description: "Locale",
      toolbar: {
        icon: "globe",
        items: [
          { value: "ja", title: "日本語" },
          { value: "en", title: "English" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { theme: "light", locale: "ja", viewport: { value: "desktop" } },
  decorators: [withProviders, withCollections, withMsw],
};

export default preview;
