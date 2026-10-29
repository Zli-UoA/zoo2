import test from "node:test";
import assert from "node:assert/strict";
import {
  summarize,
  collect,
  filesFor,
  publish,
  run,
  svg,
} from "./deployment-badges.mjs";

test("missing or partial main results remain pending; migration failure wins", () => {
  const required = ["flux/migrate", "flux/web"];
  assert.equal(summarize([], required, "app").state, "pending");
  assert.equal(
    summarize([{ context: "flux/web", state: "success" }], required, "app")
      .state,
    "pending",
  );
  assert.equal(
    summarize(
      [
        { context: "flux/web", state: "success" },
        { context: "flux/migrate", state: "failure" },
      ],
      required,
      "app",
    ).state,
    "failure",
  );
});

test("latest status replaces previous failures and unrelated checks are ignored", () => {
  const statuses = [
    { context: "kustomization/db/hash", state: "success" },
    { context: "unrelated", state: "failure" },
    { context: "kustomization/db/hash", state: "failure" },
  ];
  assert.equal(summarize(statuses, ["db"], "infra").state, "success");
  assert.equal(summarize(statuses, ["db", "web"], "infra").state, "pending");
});

test("read immutable main SHA, paginate history, and use actual Deployment status", async () => {
  const calls = [];
  const api = async (path) => {
    calls.push(path);
    if (path === "commits/main") return { sha: "a".repeat(40) };
    if (path.endsWith("statuses?per_page=100&page=1"))
      return Array.from({ length: 100 }, () => ({
        context: "other",
        state: "success",
      }));
    if (path.endsWith("statuses?per_page=100&page=2"))
      return [{ context: "flux/web", state: "success" }];
    if (path.startsWith("deployments?"))
      return [{ id: 7, sha: "b".repeat(40) }];
    if (path === "deployments/7/statuses?per_page=1")
      return [{ state: "failure", created_at: "2026-10-29T00:00:00Z" }];
    throw Error(path);
  };
  const result = await collect(api, { mode: "app", required: ["flux/web"] });
  assert.equal(result.state, "success");
  assert.equal(result.production.state, "failure");
  assert.ok(
    calls.includes(`commits/${"a".repeat(40)}/statuses?per_page=100&page=2`),
  );
  assert.match(
    filesFor(result, "org/repo")["production.svg"],
    /failure bbbbbbb/,
  );
});

test("empty deployment history is unknown, while main status remains independent", async () => {
  const result = await collect(
    async (path) => (path === "commits/main" ? { sha: "a".repeat(40) } : []),
    { mode: "app", required: ["flux/web"] },
  );
  const files = filesFor(result, "org/repo");
  assert.match(files["main.svg"], /pending aaaaaaa/);
  assert.match(files["production.svg"], /production: unknown/);
  assert.match(files["updated.svg"], /checked UTC/);
});

test("escape XML and preserve snapshot time in machine readable output", () => {
  assert.match(svg("a<&", '"injected"', "unknown"), /a&lt;&amp;/);
  assert.match(svg("a", '"injected"', "unknown"), /&quot;injected&quot;/);
  assert.equal(
    JSON.parse(
      filesFor({ state: "unknown" }, "org/repo", "2026-10-29T00:00:00Z")[
        "status.json"
      ],
    ).checked_at,
    "2026-10-29T00:00:00Z",
  );
});

test("initial publication creates an orphan branch containing only badge files", async () => {
  const writes = [];
  await publish(
    async (path, method, body) => {
      if (!method) throw Object.assign(Error("missing"), { status: 404 });
      writes.push({ path, body });
      return { sha: path === "git/trees" ? "tree" : "commit" };
    },
    { "main.svg": "<svg/>" },
  );
  assert.deepEqual(writes[1].body.parents, []);
  assert.deepEqual(writes[2].body, {
    ref: "refs/heads/deployment-badges",
    sha: "commit",
  });
});

test("existing branch updates are fast-forward only; permission errors propagate", async () => {
  const writes = [];
  await publish(async (path, method, body) => {
    if (!method) return { object: { sha: "previous" } };
    writes.push({ path, body });
    return { sha: path === "git/trees" ? "tree" : "commit" };
  }, {});
  assert.deepEqual(writes[1].body.parents, ["previous"]);
  assert.equal(writes[2].body.force, false);
  await assert.rejects(
    publish(async () => {
      throw Object.assign(Error("denied"), { status: 403 });
    }, {}),
    /denied/,
  );
});

test("collection failure replaces green with unknown and still publishes", async () => {
  let content;
  await run(
    async (path, method, body) => {
      if (path === "commits/main") throw Error("API failure");
      if (path.startsWith("git/ref/"))
        throw Object.assign(Error("missing"), { status: 404 });
      if (path === "git/trees")
        content = body.tree.find((entry) => entry.path === "main.svg").content;
      return { sha: "new" };
    },
    { mode: "app", required: ["flux/web"] },
    "org/repo",
  );
  assert.match(content, /unknown unknown/);
  assert.doesNotMatch(content, /success/);
});
