import type { Preview } from "@storybook/react-vite";
import { configure } from "storybook/test";

import "../src/styles.css";
import { withImplementation } from "./implementation";
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
    // CI runs multiple browser stories concurrently; wait for real DOM updates
    // without failing after Testing Library's default one-second deadline.
    configure({ asyncUtilTimeout: 5000 });
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
        order: ["Foundations", "Components", "Features", "Pages", "Flows"],
      },
    },
  },
  globalTypes: {
    implementation: {
      description: "UI implementation",
      toolbar: {
        icon: "component",
        items: [
          { value: "react", title: "React" },
          { value: "kotlin", title: "Kotlin (Compose)" },
        ],
        dynamicTitle: true,
      },
    },
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
          { value: "zh-Hans", title: "简体中文" },
          { value: "zh-Hant", title: "繁體中文" },
          { value: "yue", title: "粵語" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: {
    implementation: "react",
    theme: "light",
    locale: "ja",
    viewport: { value: "desktop" },
  },
  decorators: [withImplementation, withProviders, withCollections, withMsw],
};

export default preview;
