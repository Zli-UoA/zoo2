/** Shared between React story decorators and the Kotlin host. */
export const storySurfaces = {
  "Components/Dialog": "flex min-h-96 items-center justify-center bg-background p-6",
  "Components/AlertDialog": "flex min-h-96 items-center justify-center bg-background p-6",
  "Components/ControlPanel": "mx-auto max-w-xl p-4",
  "Components/Select": "w-40",
  "Components/Avatar": "bg-background p-6",
  "Components/Skeleton": "w-fit",
  "Pages/Notifications": "h-[720px] overflow-hidden bg-background",
  "Pages/BubbleChat": "h-[720px] overflow-hidden bg-background",
  "Components/Button": "bg-background p-6",
  "Components/Input": "w-[209px]",
  "Components/Textarea": "max-w-xl bg-background p-6",
  "Features/Chat/MessageRow": "mx-auto max-w-3xl bg-background py-8",
  "Features/Chat/Composer": "mx-auto max-w-3xl bg-background",
  "Features/Chat/MessageContent":
    "mx-auto max-w-2xl whitespace-pre-wrap break-words bg-background p-4 text-sm text-foreground",
  "Features/Chat/MessageCard": "mx-auto max-w-3xl bg-background p-6",
  "Features/Chat/LinkPreviews": "mx-auto max-w-4xl p-4",
} as const;
