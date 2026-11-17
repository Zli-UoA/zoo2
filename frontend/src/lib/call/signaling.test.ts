import { describe, expect, it } from "vite-plus/test";
import { readCallEvents } from "./signaling";
describe("shared call SSE", () => {
  it("handles split CRLF, UTF-8, multiline JSON and ignores invalid payloads", async () => {
    const bytes = new TextEncoder().encode(
      ': ping\r\nevent: signal\r\ndata: {"text":\r\ndata: "着信"}\r\n\r\ndata: []\n\ndata: invalid\n\n',
    );
    let offset = 0;
    const response = new Response(
      new ReadableStream({
        pull(controller) {
          if (offset === bytes.length) {
            controller.close();
            return;
          }
          controller.enqueue(bytes.slice(offset, ++offset));
        },
      }),
    );
    const events = [];
    for await (const event of readCallEvents(response)) events.push(event);
    expect(events).toEqual([{ eventType: "signal", data: { text: "着信" } }]);
  });
  it("cancels the response reader when the consumer stops", async () => {
    let cancelled = false;
    const response = new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode('data: {"ready":true}\n\n'));
        },
        cancel() {
          cancelled = true;
        },
      }),
    );
    for await (const _event of readCallEvents(response)) break;
    expect(cancelled).toBe(true);
  });
  it("rejects an unfinished oversized event", async () => {
    const events = readCallEvents(new Response("data: " + "x".repeat(65_536)));
    await expect(events.next()).rejects.toThrow("Call event too large");
  });
});
