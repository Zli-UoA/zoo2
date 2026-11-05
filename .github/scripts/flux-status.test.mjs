import assert from "node:assert/strict";
import { test } from "node:test";
import { readFile } from "node:fs/promises";
import {
  createApi,
  findStateRun,
  parseEvent,
  parseState,
  pruneStateArtifacts,
  report,
  resolveSha,
} from "./flux-status.mjs";

const sha = "a".repeat(40);
const digest = `sha256:${"b".repeat(64)}`;
const sha2 = "c".repeat(40);
const digest2 = `sha256:${"d".repeat(64)}`;
const base = "/repos/Zli-UoA/zoo2";
const runUrl = "https://github.com/Zli-UoA/zoo2/actions/runs/123";

function event(name = "zoo2", changes = {}) {
  return {
    action: `Kustomization/${name}.flux-system`,
    client_payload: {
      involvedObject: { kind: "Kustomization", namespace: "flux-system", name },
      reportingController: "kustomize-controller",
      severity: "info",
      reason: "ReconciliationSucceeded",
      timestamp: "2026-10-22T10:00:00Z",
      metadata: { digest, "release-tag": "0.0.123" },
      ...changes,
    },
  };
}

function fixture() {
  const writes = [];
  const statuses = [];
  const deployments = [];
  const deploymentStatuses = new Map();
  const api = async (path, body) => {
    if (body !== undefined) {
      writes.push({ path, body });
      if (path.startsWith(`${base}/statuses/`)) {
        statuses.unshift({ ...body, sha: path.split("/").at(-1) });
        return body;
      }
      if (path === `${base}/deployments`) {
        const value = { ...body, sha: body.ref, id: deployments.length + 1 };
        deployments.unshift(value);
        return value;
      }
      const match = path.match(/\/deployments\/(\d+)\/statuses$/);
      if (match) {
        deploymentStatuses.set(Number(match[1]), [body]);
        return body;
      }
      throw new Error(`Unexpected write: ${path}`);
    }
    if (path.startsWith("/orgs/"))
      return [
        {
          name: digest,
          metadata: { container: { tags: ["0.0.123", "0.0.125", sha] } },
        },
        { name: digest2, metadata: { container: { tags: ["0.0.124", sha2] } } },
      ];
    if (path === `${base}/commits/${sha}`) return { sha };
    if (path === `${base}/commits/${sha2}`) return { sha: sha2 };
    if (path.includes("/commits/") && path.includes("/statuses?"))
      return statuses.filter(
        (s) => s.sha === path.split("/commits/")[1].split("/")[0],
      );
    if (path.startsWith(`${base}/deployments?`)) return deployments;
    const match = path.match(/\/deployments\/(\d+)\/statuses\?/);
    if (match) return deploymentStatuses.get(Number(match[1])) ?? [];
    throw new Error(`Unexpected read: ${path}`);
  };
  return {
    api,
    writes,
    statuses,
    deployments,
    deploymentStatuses,
    runUrl,
    state: { version: 1, workloads: {} },
  };
}

test("Web completion reports the image SHA and a successful production Deployment", async () => {
  const f = fixture();
  await report(event(), f);
  assert.equal(f.writes.length, 3);
  assert.deepEqual(f.writes[0], {
    path: `${base}/statuses/${sha}`,
    body: {
      state: "success",
      context: "flux/zoo2",
      description: `zoo2: success 0.0.123 ${digest}`,
      target_url: runUrl,
    },
  });
  const deployment = f.writes[1].body;
  assert.equal(deployment.ref, sha);
  assert.equal(deployment.environment, "production");
  assert.equal(deployment.auto_merge, false);
  assert.deepEqual(deployment.required_contexts, []);
  assert.equal(deployment.payload.digest, digest);
  assert.equal(f.writes[2].body.environment_url, "https://zli.chat");
  assert.equal(f.writes[2].body.state, "success");
});

test("migration completion creates only its commit status", async () => {
  const f = fixture();
  await report(event("zoo2-migrate"), f);
  assert.equal(f.writes.length, 1);
  assert.equal(f.writes[0].body.context, "flux/zoo2-migrate");
});

for (const name of ["zoo2", "zoo2-migrate"]) {
  test(`${name} failure reports failure without a Deployment`, async () => {
    const f = fixture();
    await report(
      event(name, { severity: "error", reason: "HealthCheckFailed" }),
      f,
    );
    assert.equal(f.writes.length, 1);
    assert.equal(f.writes[0].body.state, "failure");
    assert.equal(f.writes[0].body.context, `flux/${name}`);
  });
}

test("intermediate and dependency events cannot turn a commit green", async () => {
  for (const reason of [
    "Progressing",
    "ProgressingWithRetry",
    "DependencyNotReady",
    "Succeeded",
    "HealthCheckCanceled",
  ]) {
    const result = await report(event("zoo2", { reason }), {
      api: () => assert.fail("Intermediate event must not call GitHub"),
      runUrl,
    });
    assert.equal(result.skipped, "intermediate");
  }
});

test("rejects an unexpected object, controller, action, malformed digest or timestamp", () => {
  for (const changes of [
    {
      involvedObject: {
        kind: "Kustomization",
        namespace: "other",
        name: "zoo2",
      },
    },
    { reportingController: "other" },
    { metadata: {} },
    { metadata: { digest: "sha256:invalid" } },
    { timestamp: "yesterday" },
  ])
    assert.throws(() => parseEvent(event("zoo2", changes)));
  assert.throws(() =>
    parseEvent({ ...event(), action: "Kustomization/other.flux-system" }),
  );
});

test("finds a digest and SHA on subsequent package API pages", async () => {
  const calls = [];
  const result = await resolveSha(
    async (path) => {
      calls.push(path);
      if (path.endsWith("page=1"))
        return Array.from({ length: 100 }, () => ({ name: "unrelated" }));
      return [
        {
          name: digest,
          metadata: { container: { tags: ["0.0.123", "0.0.125", sha] } },
        },
      ];
    },
    digest,
    "0.0.123",
  );
  assert.equal(result, sha);
  assert.equal(calls.length, 2);
  assert.match(calls[1], /per_page=100&page=2$/);
});

test("missing digest or SHA tag warns and skips without writing", async () => {
  for (const versions of [
    [],
    [{ name: digest, metadata: { container: { tags: ["0.0.123"] } } }],
  ]) {
    const warnings = [];
    const result = await report(event(), {
      api: async () => versions,
      runUrl,
      warn: (message) => warnings.push(message),
    });
    assert.equal(result.skipped, "missing-sha");
    assert.equal(warnings.length, 1);
  }
});

test("ambiguous SHA tags fail rather than attributing to an arbitrary commit", async () => {
  await assert.rejects(
    resolveSha(
      async () => [
        {
          name: digest,
          metadata: { container: { tags: ["0.0.123", sha, "c".repeat(40)] } },
        },
      ],
      digest,
      "0.0.123",
    ),
    /multiple/,
  );
});

test("a package or status API failure fails the run", async () => {
  await assert.rejects(
    report(event(), {
      api: async () => {
        throw new Error("GitHub API 403");
      },
      runUrl,
    }),
    /403/,
  );
  const f = fixture();
  await assert.rejects(
    report(event(), {
      ...f,
      api: async (path, body) => {
        if (body) throw new Error("GitHub API 503");
        return f.api(path);
      },
    }),
    /503/,
  );
});

test("does not post a SHA that fails repository commit validation", async () => {
  const f = fixture();
  await assert.rejects(
    report(event(), {
      ...f,
      api: async (path, body) =>
        path === `${base}/commits/${sha}`
          ? { sha: "c".repeat(40) }
          : f.api(path, body),
    }),
    /zoo2 commit/,
  );
  assert.equal(f.writes.length, 0);
});

test("rerunning the same event does not duplicate any writes", async () => {
  const f = fixture();
  await report(event(), f);
  await report(event(), f);
  assert.equal(f.writes.length, 3);
});

test("a retry completes an existing Deployment after its status API failed", async () => {
  const f = fixture();
  await assert.rejects(
    report(event(), {
      ...f,
      api: async (path, body) => {
        if (body && path.endsWith("/deployments/1/statuses"))
          throw new Error("503");
        return f.api(path, body);
      },
    }),
    /503/,
  );
  assert.equal(f.deployments.length, 1);
  await report(event(), f);
  assert.equal(f.deployments.length, 1);
  assert.equal(f.deploymentStatuses.get(1)[0].state, "success");
});

test("a delayed older success cannot overwrite a newer failure on the commit", async () => {
  const f = fixture();
  await report(
    event("zoo2", {
      severity: "error",
      reason: "HealthCheckFailed",
      timestamp: "2026-10-22T11:00:00Z",
    }),
    f,
  );
  assert.equal((await report(event(), f)).skipped, "older-event");
  assert.equal(f.writes.length, 1);
});

test("an older image event cannot create a Deployment after a newer production result", async () => {
  const f = fixture();
  f.deployments.push({
    sha: "c".repeat(40),
    payload: { source: "flux/zoo2", timestamp: "2026-10-22T11:00:00Z" },
  });
  assert.equal((await report(event(), f)).skipped, "older-deployment");
  assert.equal(f.writes.length, 1);
});

test("HTTP errors surface without logging the token or response body", async () => {
  const api = createApi(
    "secret-token",
    async () => new Response("private data", { status: 403 }),
  );
  await assert.rejects(api("/example"), (error) => {
    assert.match(error.message, /GitHub API 403/);
    assert.doesNotMatch(error.message, /secret-token|private data/);
    return true;
  });
});

test("API client sends JSON and authorization with the fixed GitHub host", async () => {
  let request;
  const api = createApi("test-token", async (url, options) => {
    request = { url, options };
    return Response.json({ id: 1 });
  });
  assert.deepEqual(await api("/example", { state: "success" }), { id: 1 });
  assert.equal(request.url, "https://api.github.com/example");
  assert.equal(request.options.headers.Authorization, "Bearer test-token");
  assert.equal(request.options.body, '{"state":"success"}');
});

test("later successful reconciliation reuses the Deployment and advances its durable watermark", async () => {
  const f = fixture();
  await report(event(), f);
  await report(event("zoo2", { timestamp: "2026-10-22T12:00:00Z" }), f);
  assert.equal(f.writes.length, 3);
  assert.equal(f.deployments.length, 1);
  assert.equal(f.state.workloads.zoo2.timestamp, "2026-10-22T12:00:00Z");
  // A new runner restores the artifact rather than keeping in-process state.
  const restored = parseState(JSON.stringify(f.state));
  const result = await report(
    event("zoo2", {
      timestamp: "2026-10-22T11:00:00Z",
      severity: "error",
      reason: "HealthCheckFailed",
    }),
    { ...f, state: restored },
  );
  assert.equal(result.skipped, "older-event");
  assert.equal(f.writes.length, 3);
});

test("repeated failures do not exhaust the commit status limit, but still advance the watermark", async () => {
  const f = fixture();
  for (const timestamp of ["2026-10-22T10:00:00Z", "2026-10-22T12:00:00Z"]) {
    await report(
      event("zoo2-migrate", {
        severity: "error",
        reason: "HealthCheckFailed",
        timestamp,
      }),
      f,
    );
  }
  assert.equal(f.writes.length, 1);
  const result = await report(
    event("zoo2-migrate", { timestamp: "2026-10-22T11:00:00Z" }),
    f,
  );
  assert.equal(result.skipped, "older-event");
  assert.equal(f.writes.length, 1);
});

test("different digests share the watermark and delayed events cannot replace a newer rollout", async () => {
  const f = fixture();
  await report(
    event("zoo2", {
      metadata: { digest: digest2, "release-tag": "0.0.124" },
      timestamp: "2026-10-22T12:00:00Z",
    }),
    f,
  );
  assert.equal((await report(event(), f)).skipped, "older-event");
  assert.equal(f.deployments.length, 1);
  assert.equal(f.deployments[0].sha, sha2);
});

test("A to B to A records a real rollback rather than reusing historical A", async () => {
  const f = fixture();
  await report(event(), f);
  await report(
    event("zoo2", {
      metadata: { digest: digest2, "release-tag": "0.0.124" },
      timestamp: "2026-10-22T11:00:00Z",
    }),
    f,
  );
  await report(event("zoo2", { timestamp: "2026-10-22T12:00:00Z" }), f);
  assert.deepEqual(
    f.deployments.map((d) => d.sha),
    [sha, sha2, sha],
  );
  assert.equal(f.deployments[0].payload.timestamp, "2026-10-22T12:00:00Z");
});

test("invalid watermark fails closed rather than silently losing event ordering", () => {
  for (const text of [
    "null",
    "{}",
    '{"version":1,"workloads":[]}',
    '{"version":1,"workloads":{"zoo2":{"timestamp":"bad"}}}',
  ])
    assert.throws(() => parseState(text));
});

test("artifact restoration accepts only this workflow on the trusted default branch", async () => {
  const trusted = {
    event: "repository_dispatch",
    head_branch: "main",
    path: ".github/workflows/flux-status.yaml",
    head_repository: { full_name: "Zli-UoA/zoo2" },
  };
  const runs = {
    1: { ...trusted, event: "pull_request" },
    2: { ...trusted, path: ".github/workflows/ci.yml" },
    3: { ...trusted, head_repository: { full_name: "other/zoo2" } },
    4: trusted,
  };
  const result = await findStateRun(async (path) => {
    if (path.includes("/artifacts?"))
      return {
        artifacts: [1, 2, 3, 4].map((id) => ({ workflow_run: { id } })),
      };
    return runs[Number(path.split("/").at(-1))];
  });
  assert.equal(result, 4);
  assert.equal(await findStateRun(async () => ({ artifacts: [] })), null);
  await assert.rejects(
    findStateRun(async () => {
      throw new Error("403");
    }),
    /403/,
  );
});

test("workflow serializes all images, retains pending events and restores/saves ordering state", async () => {
  const workflow = await readFile(
    new URL("../workflows/flux-status.yaml", import.meta.url),
    "utf8",
  );
  assert.match(workflow, /group: flux-zoo2-production-status/);
  assert.match(workflow, /cancel-in-progress: false/);
  assert.match(workflow, /queue: max/);
  assert.doesNotMatch(workflow, /group:.*digest/);
  assert.ok(
    workflow.indexOf("--state-run") <
      workflow.indexOf("run: node .github/scripts/flux-status.mjs"),
  );
  assert.ok(
    workflow.indexOf("run: node .github/scripts/flux-status.mjs") <
      workflow.indexOf("name: Save event watermark"),
  );
});

test("watermark restoration orders all artifact pages by snapshot creation, including retries of old runs", async () => {
  const trusted = {
    event: "repository_dispatch",
    head_branch: "main",
    path: ".github/workflows/flux-status.yaml",
    head_repository: { full_name: "Zli-UoA/zoo2" },
  };
  const calls = [];
  const result = await findStateRun(async (path) => {
    calls.push(path);
    if (path.includes("/artifacts?") && path.endsWith("page=1"))
      return {
        artifacts: [
          {
            id: 200,
            created_at: "2026-10-22T10:00:00Z",
            workflow_run: { id: 20 },
          },
          ...Array.from({ length: 99 }, () => ({ expired: true })),
        ],
      };
    if (path.includes("/artifacts?"))
      return {
        artifacts: [
          {
            id: 300,
            created_at: "2026-10-22T12:00:00Z",
            workflow_run: { id: 11 },
          },
        ],
      };
    return trusted;
  });
  assert.equal(result, 11);
  assert.equal(calls.filter((path) => path.includes("/artifacts?")).length, 2);
});

test("state restoration accepts main ref suffix and rejects other refs", async () => {
  for (const [path, expected] of [
    [".github/workflows/flux-status.yaml@main", 10],
    [".github/workflows/flux-status.yaml@feature", null],
    [".github/workflows/flux-status.yaml@main/extra", null],
  ]) {
    assert.equal(
      await findStateRun(async (url) =>
        url.includes("/artifacts?")
          ? { artifacts: [{ workflow_run: { id: 10 } }] }
          : {
              event: "repository_dispatch",
              head_branch: "main",
              path,
              head_repository: { full_name: "Zli-UoA/zoo2" },
            },
      ),
      expected,
    );
  }
});

test("snapshot pruning retains latest two trusted snapshots including older-run retries", async () => {
  const artifacts = [
    { id: 4, created_at: "2026-10-23T04:00:00Z", workflow_run: { id: 1 } },
    { id: 3, created_at: "2026-10-23T03:00:00Z", workflow_run: { id: 3 } },
    { id: 2, created_at: "2026-10-23T02:00:00Z", workflow_run: { id: 2 } },
    { id: 5, created_at: "2026-10-23T05:00:00Z", workflow_run: { id: 5 } },
  ];
  const deleted = [];
  await pruneStateArtifacts(async (url, body, method) => {
    if (method === "DELETE") {
      deleted.push(url);
      return null;
    }
    if (url.includes("/artifacts?")) return { artifacts };
    return {
      event: url.endsWith("/5") ? "pull_request" : "repository_dispatch",
      head_branch: "main",
      path: ".github/workflows/flux-status.yaml@main",
      head_repository: { full_name: "Zli-UoA/zoo2" },
    };
  });
  assert.deepEqual(deleted, [`${base}/actions/artifacts/2`]);
});

test("failed snapshot pruning surfaces the API error without deleting newer snapshots", async () => {
  const calls = [];
  await assert.rejects(
    pruneStateArtifacts(async (url, body, method) => {
      if (url.includes("/artifacts?"))
        return {
          artifacts: [3, 2, 1].map((id) => ({ id, workflow_run: { id } })),
        };
      if (method === "DELETE") {
        calls.push(url);
        throw new Error("403");
      }
      return {
        event: "repository_dispatch",
        head_branch: "main",
        path: ".github/workflows/flux-status.yaml",
        head_repository: { full_name: "Zli-UoA/zoo2" },
      };
    }),
    /403/,
  );
  assert.deepEqual(calls, [`${base}/actions/artifacts/1`]);
});

test("a digest without the requested release tag cannot identify a deployed commit", async () => {
  const warnings = [];
  const f = fixture();
  const result = await report(
    event("zoo2", { metadata: { digest, "release-tag": "0.0.999" } }),
    {
      ...f,
      warn: (message) => warnings.push(message),
    },
  );
  assert.equal(result.skipped, "missing-sha");
  assert.equal(warnings.length, 1);
  assert.equal(f.writes.length, 0);
});

test("new release of the same SHA and digest creates a new deployment, and its retries are deduplicated", async () => {
  const f = fixture();
  await report(event(), f);
  const redeploy = event("zoo2", {
    timestamp: "2026-10-22T11:00:00Z",
    metadata: { digest, "release-tag": "0.0.125" },
  });
  await report(redeploy, f);
  assert.equal(f.deployments.length, 2);
  assert.equal(f.deployments[0].payload.releaseTag, "0.0.125");
  assert.equal(f.deployments[0].sha, sha);
  const count = f.writes.length;
  await report(redeploy, f);
  assert.equal(f.writes.length, count);
});

test("release tags are mandatory production release identifiers", () => {
  for (const tag of [
    undefined,
    "latest",
    "pr-1-abc",
    "0.0.0",
    "0.0.01",
    "1.0.123",
    "0.0.123;echo x",
  ]) {
    assert.throws(
      () =>
        parseEvent(event("zoo2", { metadata: { digest, "release-tag": tag } })),
      /release tag/,
    );
  }
});

for (const name of ["zoo2", "zoo2-migrate"]) {
  test(`${name}: a success with the same timestamp as a processed failure cannot override it`, async () => {
    const warnings = [];
    const f = fixture();
    const failure = event(name, { severity: "error", reason: "HealthCheckFailed" });
    await report(failure, f);
    const result = await report(event(name), {
      ...f,
      warn: (message) => warnings.push(message),
    });
    assert.equal(result.skipped, "ambiguous-same-time");
    assert.equal(warnings.length, 1);
    assert.equal(f.writes.length, 1);
    assert.equal(f.statuses[0].state, "failure");
    assert.equal(f.deployments.length, 0);
    assert.deepEqual(f.state.workloads[name], {
      timestamp: "2026-10-22T10:00:00Z",
      state: "failure",
    });
    // A later reconciliation still reports the recovery.
    await report(event(name, { timestamp: "2026-10-22T10:00:01Z" }), f);
    assert.equal(f.statuses[0].state, "success");
    assert.equal(f.deployments.length, name === "zoo2" ? 1 : 0);
  });
}

test("a failure with the same timestamp as a processed success is still recorded", async () => {
  const f = fixture();
  await report(event(), f);
  await report(event("zoo2", { severity: "error", reason: "HealthCheckFailed" }), f);
  assert.equal(f.statuses[0].state, "failure");
  assert.equal(f.state.workloads.zoo2.state, "failure");
  // Rerunning the earlier success cannot turn the commit green again.
  assert.equal((await report(event(), f)).skipped, "ambiguous-same-time");
  assert.equal(f.statuses[0].state, "failure");
});

test("rerunning the same failure or success event stays idempotent", async () => {
  for (const changes of [{}, { severity: "error", reason: "HealthCheckFailed" }]) {
    const f = fixture();
    await report(event("zoo2", changes), f);
    const count = f.writes.length;
    await report(event("zoo2", changes), f);
    assert.equal(f.writes.length, count);
  }
});

test("watermarks accept legacy entries without state and reject unknown states", () => {
  assert.doesNotThrow(() =>
    parseState('{"version":1,"workloads":{"zoo2":{"timestamp":"2026-10-22T10:00:00Z"}}}'),
  );
  assert.throws(() =>
    parseState(
      '{"version":1,"workloads":{"zoo2":{"timestamp":"2026-10-22T10:00:00Z","state":"pending"}}}',
    ),
  );
});
