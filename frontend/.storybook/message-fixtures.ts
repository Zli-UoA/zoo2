import { storyDeploymentCard } from "./app-fixtures";
export const githubFeedCards = [
  { body: "Deployment to storybook-pr-271 by Kai-Z-JP", card: storyDeploymentCard },
  {
    body: "1 new commit pushed to main by Kai-Z-JP",
    card: {
      ...storyDeploymentCard,
      title: "Zli-UoA/zoo2 · 103eb20",
      color: "#24292f",
      fields: [
        {
          title: "103eb20",
          value: "fix: preserve queued webhooks while Zoo credentials are revoked (#271)",
          url: "https://github.com/Zli-UoA/zoo2/commit/103eb20",
        },
      ],
    },
  },
  {
    body: "Pull request merged by Kai-Z-JP",
    card: {
      ...storyDeploymentCard,
      title: "#271 fix: Zoo 資格情報の失効中も Bot の受信キューを保持する",
      color: "#8250df",
      url: "https://github.com/Zli-UoA/zoo2/pull/271",
      fields: [],
    },
  },
];
export const reactionKitchenCatalog = {
  emojis: [
    ["1f600", "😀", "grinning face"],
    ["1f431", "🐱", "cat face"],
  ] as [string, string, string][],
  dates: ["20260101"],
  pairs: { "0_1": 0 },
};
