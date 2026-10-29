import test from "node:test";
import assert from "node:assert/strict";
import {
  summarize,
  collect,
  endpoint,
  filesFor,
  makeApi,
  makeGistPublisher,
  validateConfig,
  jobSummary,
  run,
} from "./deployment-badges.mjs";

const gistId = "1".repeat(32);
const config = {
  mode: "app",
  prefix: "authz",
  required: ["flux/migrate", "flux/web"],
  gistId,
};
const at = "2026-10-29T03:00:00Z";
const fullSha = "a".repeat(40);
const badge = (files, name) => JSON.parse(files[name].content);

test("pending for missing/partial results, failure takes precedence, all-success turns green", () => {
  const statuses = [{ context: "flux/web", state: "success" }];
  assert.equal(summarize([], config.required, "app").state, "pending");
  assert.equal(summarize(statuses, config.required, "app").state, "pending");
  assert.equal(
    summarize(
      [...statuses, { context: "flux/migrate", state: "error" }],
      config.required,
      "app",
    ).state,
    "failure",
  );
  assert.equal(
    summarize(
      [...statuses, { context: "flux/migrate", state: "success" }],
      config.required,
      "app",
    ).state,
    "success",
  );
});

test("infra uses latest status per resource, ignoring unrelated statuses", () => {
  const statuses = [
    { context: "kustomization/db/hash", state: "success" },
    { context: "ci", state: "failure" },
    { context: "kustomization/db/hash", state: "failure" },
  ];
  assert.equal(summarize(statuses, ["db"], "infra").state, "success");
  assert.equal(summarize(statuses, ["db", "web"], "infra").state, "pending");
});

test("collection fixes main SHA, paginates statuses and preserves separate production result", async () => {
  const calls = [];
  const result = await collect(
    async (path) => {
      calls.push(path);
      if (path === "commits/main") return { sha: fullSha };
      if (path.endsWith("statuses?per_page=100&page=1"))
        return Array.from({ length: 100 }, () => ({
          context: "unrelated",
          state: "success",
        }));
      if (path.endsWith("statuses?per_page=100&page=2"))
        return [{ context: "flux/web", state: "success" }];
      if (path.startsWith("deployments?"))
        return [{ id: 7, sha: "b".repeat(40) }];
      if (path === "deployments/7/statuses?per_page=1")
        return [{ state: "failure", created_at: at }];
      throw Error(path);
    },
    { ...config, required: ["flux/web"] },
  );
  assert.equal(result.state, "success");
  assert.equal(result.production.state, "failure");
  assert.ok(calls.includes(`commits/${fullSha}/statuses?per_page=100&page=2`));
  assert.equal(
    badge(filesFor(result, config, at), "authz-production.json").message,
    "failure bbbbbbb",
  );
});

test("public files contain only endpoint fields, short SHAs and collection time", () => {
  const result = {
    sha: fullSha,
    state: "success",
    privateField: "secret-source",
    entries: [
      {
        name: "private-resource",
        description: "private-log",
        target_url: "https://private.example",
      },
    ],
    production: { sha: "b".repeat(40), state: "success" },
  };
  const files = filesFor(result, config, at);
  assert.deepEqual(Object.keys(files).sort(), [
    "authz-main.json",
    "authz-production.json",
    "authz-updated.json",
  ]);
  for (const file of Object.values(files)) {
    const value = JSON.parse(file.content);
    assert.deepEqual(Object.keys(value).sort(), [
      "color",
      "label",
      "message",
      "schemaVersion",
    ]);
    assert.equal(value.schemaVersion, 1);
    assert.ok(value.message);
    assert.doesNotMatch(
      file.content,
      /secret-source|private-|aaaaaaaaaaaaaaaa|bbbbbbbbbbbbbbbb/,
    );
  }
  assert.equal(badge(files, "authz-main.json").message, "success aaaaaaa");
  assert.equal(badge(files, "authz-updated.json").message, "2026-10-29 03:00");
});

test("missing production is unknown, infra emits no production badge, malformed SHAs never escape", () => {
  assert.equal(
    badge(filesFor({ state: "pending" }, config, at), "authz-production.json")
      .message,
    "unknown",
  );
  const infraFiles = filesFor(
    { state: "success", sha: "private-value" },
    { mode: "infra", prefix: "infra" },
    at,
  );
  assert.deepEqual(Object.keys(infraFiles).sort(), [
    "infra-main.json",
    "infra-updated.json",
  ]);
  assert.equal(badge(infraFiles, "infra-main.json").message, "success unknown");
  assert.equal(
    endpoint("Flux main", "unknown", "unrecognized").color,
    "#6e7781",
  );
});

test("config rejects wrong repository prefixes, missing Gist IDs and empty required sets", () => {
  validateConfig(config, "Zli-UoA/AuthZ");
  assert.throws(
    () => validateConfig({ ...config, prefix: "zoo2" }, "Zli-UoA/AuthZ"),
    /prefix/,
  );
  assert.throws(
    () => validateConfig({ ...config, gistId: "" }, "Zli-UoA/AuthZ"),
    /Gist ID/,
  );
  assert.throws(
    () => validateConfig({ ...config, required: [] }, "Zli-UoA/AuthZ"),
    /configuration/,
  );
  assert.throws(() => makeGistPublisher(gistId, ""), /BADGE_GIST_TOKEN/);
});

test("repository and Gist requests use different credentials and no Git write API", async () => {
  const calls = [];
  const request = async (url, options) => {
    calls.push({ url, options });
    return new Response(JSON.stringify({ sha: fullSha }), { status: 200 });
  };
  await makeApi("Zli-UoA/AuthZ", "repo-token", request)("commits/main");
  await makeGistPublisher(gistId, "gist-token", { request })(
    filesFor({ state: "pending" }, config, at),
  );
  assert.equal(calls[0].options.headers.Authorization, "Bearer repo-token");
  assert.equal(calls[1].options.headers.Authorization, "Bearer gist-token");
  assert.equal(calls[1].url, `https://api.github.com/gists/${gistId}`);
  assert.equal(calls[1].options.method, "PATCH");
  assert.deepEqual(Object.keys(JSON.parse(calls[1].options.body)), ["files"]);
  assert.ok(calls.every(({ url }) => !url.includes("/git/")));
});

test("interleaved updates preserve other repositories and unrelated Gist files", async () => {
  const gist = { "unrelated.txt": { content: "preserve me" } };
  const request = async (_url, options) => {
    Object.assign(gist, JSON.parse(options.body).files);
    return new Response("{}");
  };
  const publish = makeGistPublisher(gistId, "gist-token", { request });
  await Promise.all([
    publish(filesFor({ state: "success", sha: fullSha }, config, at)),
    publish(
      filesFor(
        { state: "failure", sha: fullSha },
        { mode: "app", prefix: "zoo2" },
        at,
      ),
    ),
    publish(
      filesFor(
        { state: "pending", sha: fullSha },
        { mode: "infra", prefix: "infra" },
        at,
      ),
    ),
  ]);
  assert.equal(Object.keys(gist).length, 9);
  assert.equal(gist["unrelated.txt"].content, "preserve me");
  assert.equal(badge(gist, "authz-main.json").message, "success aaaaaaa");
  assert.equal(badge(gist, "zoo2-main.json").message, "failure aaaaaaa");
});

test("retry 429 and 5xx, respecting bounded Retry-After, then succeed", async () => {
  const sleeps = [];
  let calls = 0;
  await makeGistPublisher(gistId, "token", {
    request: async () => {
      calls++;
      return new Response("{}", {
        status: calls === 1 ? 429 : calls === 2 ? 503 : 200,
        headers: { "retry-after": "999" },
      });
    },
    wait: async (ms) => {
      sleeps.push(ms);
    },
  })({});
  assert.equal(calls, 3);
  assert.deepEqual(sleeps, [30000, 30000]);
});

test("persistent HTTP failures stop at three attempts, auth failures do not retry or leak response", async () => {
  let calls = 0;
  await assert.rejects(
    makeGistPublisher(gistId, "token", {
      request: async () => {
        calls++;
        return new Response("private error body", { status: 503 });
      },
      wait: async () => {},
    })({}),
    /^Error: Gist update failed: HTTP 503$/,
  );
  assert.equal(calls, 3);
  calls = 0;
  await assert.rejects(
    makeGistPublisher(gistId, "token", {
      request: async () => {
        calls++;
        return new Response("secret token body", { status: 403 });
      },
      wait: async () => {},
    })({}),
    /^Error: Gist update failed: HTTP 403$/,
  );
  assert.equal(calls, 1);
});

test("network failures retry with bounded backoff and stop after three attempts", async () => {
  const sleeps = [];
  let calls = 0;
  await assert.rejects(
    makeGistPublisher(gistId, "token", {
      request: async () => {
        calls++;
        throw Error("sensitive network detail");
      },
      wait: async (ms) => {
        sleeps.push(ms);
      },
    })({}),
    /^Error: Gist update failed after three network attempts$/,
  );
  assert.equal(calls, 3);
  assert.deepEqual(sleeps, [1000, 2000]);
});

test("source API failure publishes unknown while a Gist write failure fails the run", async () => {
  let files;
  await run(
    async () => {
      throw Error("private source details");
    },
    async (value) => {
      files = value;
    },
    config,
    { now: at },
  );
  assert.equal(badge(files, "authz-main.json").message, "unknown unknown");
  assert.equal(badge(files, "authz-production.json").message, "unknown");
  await assert.rejects(
    run(
      async () => {
        throw Error("source failed");
      },
      async () => {
        throw Error("Gist failed");
      },
      config,
    ),
    /Gist failed/,
  );
});

test("resource details and full SHAs stay in the private Job Summary", () => {
  const summary = jobSummary(
    {
      sha: fullSha,
      state: "success",
      entries: [{ name: "db|name\nline", state: "success", updated_at: at }],
    },
    at,
  );
  assert.ok(summary.includes(fullSha));
  assert.ok(summary.includes("db name line"));
  assert.ok(summary.includes("Last event (UTC)"));
});
