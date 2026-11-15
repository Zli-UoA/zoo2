import type { Preview } from "@storybook/react-vite";
import { configure } from "storybook/test";

import "../src/styles.css";
import {
  ENCRYPTED_CHANNEL_STORAGE_PREFIX,
  resetSeenEncryptedChannels,
} from "../src/lib/e2ee/encryptionGuard";
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
  beforeEach: () => {
    // 暗号化チャンネルのストーリーが覚えた「暗号化と見たチャンネル」を、ほかのストーリーに持ち込まない
    // （同じチャンネル ID の平文チャンネルが格下げと判定され、送信が止まる）。
    resetSeenEncryptedChannels();
    try {
      for (const key of Object.keys(localStorage)) {
        if (key.startsWith(ENCRYPTED_CHANNEL_STORAGE_PREFIX)) localStorage.removeItem(key);
      }
    } catch {
      // localStorage を使えない環境では、覚えてもいない。
    }
  },
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
          { value: "ko-KR", title: "한국어" },
          { value: "ko-KP", title: "조선말 (문화어)" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: {
    theme: "light",
    locale: "ja",
    viewport: { value: "desktop" },
  },
  decorators: [withProviders, withCollections, withMsw],
};

export default preview;
