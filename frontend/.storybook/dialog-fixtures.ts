export const dialogSamples = {
  dialog: {
    trigger: "ダイアログを開く",
    title: "チャンネルを削除しますか？",
    description: "この操作は取り消せません。チャンネルの履歴もすべて削除されます。",
    cancel: "キャンセル",
    confirm: "削除する",
  },
  alert: {
    trigger: "メッセージを削除",
    title: "メッセージを削除しますか？",
    description:
      "この操作は取り消せません。削除したメッセージはチャンネルのメンバーからも見えなくなります。",
    cancel: "キャンセル",
    confirm: "削除する",
  },
} as const;
