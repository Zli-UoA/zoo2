import type { Decorator } from "@storybook/react-vite";
import { KotlinPreview } from "./KotlinPreview";
import { kotlinOptions } from "./kotlin-stories";

export const withImplementation: Decorator = (Story, context) => {
  if (context.globals.implementation !== "kotlin") return <Story />;
  const options = kotlinOptions(context);
  return options ? (
    <KotlinPreview {...options} key={context.id} />
  ) : (
    <div role="status" className="p-6 text-sm" data-kotlin-story-status="unavailable">
      このStoryのKotlin表示は未登録です。上部の切り替えでReact表示に戻れます。
    </div>
  );
};
