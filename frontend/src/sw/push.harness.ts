/**
 * `push.browser.test.ts` が登録する、テスト用の Service Worker（module）。
 *
 * 本番の `sw-push.js` と同じ `push.ts` を読み、`message` で渡された Push の `e2ee` を
 * `zooDecryptPush` で復号して、結果を MessagePort に返す。
 */
import "./push";

interface ExtendableMessageEvent extends MessageEvent {
  waitUntil(promise: Promise<unknown>): void;
}
const scope = self as unknown as {
  addEventListener(type: "message", listener: (event: ExtendableMessageEvent) => void): void;
  addEventListener(
    type: "install" | "activate",
    listener: (event: { waitUntil(promise: Promise<unknown>): void }) => void,
  ): void;
  skipWaiting(): Promise<void>;
  clients: { claim(): Promise<void> };
  zooDecryptPush?: (e2ee: unknown) => Promise<string | null>;
};

scope.addEventListener("install", (event) => event.waitUntil(scope.skipWaiting()));
scope.addEventListener("activate", (event) => event.waitUntil(scope.clients.claim()));
scope.addEventListener("message", (event) => {
  const port = event.ports[0];
  event.waitUntil(
    (scope.zooDecryptPush?.(event.data) ?? Promise.resolve(null)).then(
      (body) => port?.postMessage({ body }),
      (error: unknown) => port?.postMessage({ body: null, error: String(error) }),
    ),
  );
});
