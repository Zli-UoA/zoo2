import type { Preview } from "@storybook/react-vite";

import "../src/styles.css";
import { withMsw, withProviders } from "./decorators";

const preview: Preview = {
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
  initialGlobals: { theme: "light", locale: "ja" },
  decorators: [withProviders, withMsw],
};

export default preview;
