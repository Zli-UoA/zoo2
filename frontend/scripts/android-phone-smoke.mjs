// Execute the shipped bundle without Node globals: WebView must boot before any call is made.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
const messages = [];
const window = { ZooNativePhone: { postMessage: (message) => messages.push(JSON.parse(message)) } };
vm.runInNewContext(
  await readFile(new URL("../build/android-phone/engine.js", import.meta.url), "utf8"),
  { window },
  { timeout: 1000 },
);
assert.deepEqual(messages, [{ event: "loaded" }]);
assert.equal(typeof window.ZooPhone.execute, "function");
window.ZooPhone.execute(1, "test-token", "close", {});
await new Promise(setImmediate);
assert.deepEqual(messages.at(-1), {
  token: "test-token",
  event: "result",
  payload: { id: 1, ok: true },
});
console.log("Android phone bundle boots without Node globals and closes successfully.");
