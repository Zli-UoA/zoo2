/*
 * dev サーバー用の空の /sw-push.js（public/sw.js が importScripts で読む）。
 *
 * `pnpm build` が、src/sw/push.ts から作った本物（暗号化チャンネルの Push の復号）で
 * dist/client/sw-push.js を置き換える（vite.sw-push.config.ts）。これは `self.zooDecryptPush` を
 * 置かないので、Push は復号せずに「新しいメッセージ」のまま出る。
 */
// 復号しない（public/sw.js は、これがなければ「新しいメッセージ」のまま出す）。
self.zooDecryptPush = undefined;
