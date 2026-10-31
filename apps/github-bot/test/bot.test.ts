import { test } from "node:test";
import assert from "node:assert/strict";
import { verify, randomUUID } from "node:crypto";
import { fixture, job, command, pair } from "./helpers.ts";
import { GitHubError } from "../src/github.ts";
import { preview } from "../src/previews.ts";
import { subscription, words, permits } from "../src/subscriptions.ts";
import { ZooError } from "../../../sdk/src/index.ts";
import type { Json } from "../src/types.ts";

test("GitHub repository writes are impossible, including Zoo thread replies and action events", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  for (const [method, path] of [
    ["POST", "/repos/acme/project/issues/7/comments"],
    ["PATCH", "/repos/acme/project/issues/7"],
    ["PUT", "/repos/acme/project/pulls/8/merge"],
    ["DELETE", "/repos/acme/project/issues/7"],
    ["POST", "/graphql"],
  ])
    await assert.rejects(
      f.github.api(path, "token", method, { query: "mutation { deleteIssue }" }),
      /writes are disabled/,
    );
  await f.bot.zooEvent({
    ...command(""),
    type: "message.created",
    data: { id: "reply", parent_id: "thread", content: "Close this PR" },
  });
  await f.bot.zooEvent({ ...command(""), type: "action.invoked", data: { action_id: "merge" } });
  await f.bot.zooEvent(command("close acme/project#7"));
  assert.equal(f.remote.readRequests.length, 0);
  const jwt = f.github.jwt(),
    [header, claims, signature] = jwt.split(".");
  assert.equal(
    verify(
      "RSA-SHA256",
      Buffer.from(`${header}.${claims}`),
      pair.publicKey,
      Buffer.from(signature, "base64url"),
    ),
    true,
  );
  const payload = JSON.parse(Buffer.from(claims, "base64url").toString());
  assert.equal(payload.iss, "42");
  assert.ok(payload.exp - payload.iat <= 600);
});

test("PR updates edit one parent and reply in its thread, with merge reflected in the current parent", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  await f.bot.githubEvent(
    job("pull_request", {
      action: "opened",
      pull_request: { ...f.remote.pr, requested_reviewers: [{ login: "alice" }] },
    }),
  );
  assert.equal(f.zoo.messages.size, 1);
  const root = f.store.thread("channel", { repo: "acme/project", kind: "pull", number: "8" })!;
  assert.match(f.zoo.messages.get(root.message)!.content, /@alice_zoo/);
  f.remote.pr = { ...f.remote.pr, state: "closed", merged: true, title: "New title" };
  const closed = job("pull_request", { action: "closed", pull_request: f.remote.pr });
  await f.bot.githubEvent(closed);
  await f.bot.githubEvent(closed);
  assert.equal(f.zoo.messages.size, 2);
  assert.match(f.zoo.messages.get(root.message)!.content, /merged/);
  assert.equal(f.zoo.posts.filter((m) => m.id === root.message).length, 1);
  assert.match(f.zoo.messages.get(root.message)!.content, /New title/);
  const reply = [...f.zoo.messages.values()].find((m) => m.parent_id === root.message)!;
  assert.match(reply.content, /merged/);
  assert.equal(reply.broadcast, true);
  // A late opened webhook cannot replace the parent with its old state.
  await f.bot.githubEvent(
    job("pull_request", {
      action: "opened",
      pull_request: { ...f.remote.pr, state: "open", merged: false },
    }),
  );
  assert.match(f.zoo.messages.get(root.message)!.content, /merged/);
  assert.ok(
    f.remote.readRequests.every(
      (r) => r.method === "GET" || new URL(r.url).pathname.endsWith("/access_tokens"),
    ),
  );
});

test("GitHub comments create, edit, and delete the same Zoo reply; stale deliveries read the canonical comment", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  await f.bot.githubEvent(job("issues", { action: "opened", issue: f.remote.issue }));
  const event = job("issue_comment", {
    action: "created",
    issue: f.remote.issue,
    comment: { id: 99, body: "stale body" },
  });
  await f.bot.githubEvent(event);
  const thread = f.store.thread("channel", { repo: "acme/project", kind: "issue", number: "7" })!;
  const reply = [...f.zoo.messages.values()].find((m) => m.parent_id === thread.message)!;
  assert.match(reply.content, /Current comment/);
  assert.doesNotMatch(reply.content, /stale body/);
  assert.match(reply.content, /#issuecomment-99/);
  f.remote.comment!.body = "Edited @alice";
  await f.bot.githubEvent(
    job("issue_comment", {
      action: "edited",
      issue: f.remote.issue,
      comment: { id: 99, body: "old edit" },
    }),
  );
  assert.equal(f.zoo.messages.size, 2);
  assert.match(f.zoo.messages.get(reply.id)!.content, /Edited ＠alice/);
  assert.match(f.zoo.messages.get(reply.id)!.content, /@alice_zoo/);
  assert.equal(f.zoo.posts.filter((m) => m.id === reply.id).length, 1);
  f.remote.comment = null;
  await f.bot.githubEvent(
    job("issue_comment", { action: "deleted", issue: f.remote.issue, comment: { id: 99 } }),
  );
  assert.equal(f.zoo.messages.size, 1);
  assert.deepEqual(f.zoo.deleted, [reply.id]);
  await f.bot.githubEvent(event);
  assert.equal(f.zoo.messages.size, 1, "A late create must not resurrect a deleted GitHub comment");
});

test("Zoo transient unfurl failures remain queued and recover, while forbidden links finish", async (t) => {
  for (const status of [429, 503, 403]) {
    const f = fixture();
    t.after(() => f.store.close());
    const request = f.zoo.request.bind(f.zoo);
    let attempts = 0;
    f.zoo.request = async (method, path, body) => {
      if (path.endsWith("/unfurls") && ++attempts === 1)
        throw new ZooError(status, "Unfurl failed");
      return request(method, path, body);
    };
    const event = {
      ...command(""),
      type: "link.shared",
      data: { id: "message", links: ["https://github.com/acme/project/issues/7"] },
    };
    f.store.enqueue(event.id, "zoo", event.type, event);
    await f.bot.tick();
    const queued = f.store.db.prepare("SELECT status,attempts FROM jobs WHERE id=?").get(event.id)!;
    assert.equal(queued.status, status === 403 ? "done" : "queued");
    assert.equal(
      f.zoo.calls.some((c) => c.path === "/surfaces"),
      status === 403,
    );
    if (status !== 403) {
      f.store.db.prepare("UPDATE jobs SET available=0 WHERE id=?").run(event.id);
      await f.bot.tick();
      assert.equal(attempts, 2);
      assert.equal(f.zoo.calls.filter((c) => c.path.endsWith("/unfurls")).length, 1);
      const finished = f.store.db
        .prepare("SELECT status,attempts FROM jobs WHERE id=?")
        .get(event.id)!;
      assert.equal(finished.status, "done");
      assert.equal(finished.attempts, 2);
    }
  }
});

test("review dismissal edits its existing reply and Checks changes edit the PR metadata", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.store.subscribe({ repo: "acme/project", channel: "channel", features: ["pulls", "reviews"] });
  f.remote.reviews = [f.remote.review];
  await f.bot.githubEvent(job("pull_request", { action: "opened", pull_request: f.remote.pr }));
  await f.bot.githubEvent(
    job("pull_request_review", {
      action: "submitted",
      pull_request: f.remote.pr,
      review: f.remote.review,
    }),
  );
  const root = f.store.thread("channel", { repo: "acme/project", kind: "pull", number: "8" })!,
    reply = [...f.zoo.messages.values()].find((m) => m.parent_id === root.message)!;
  assert.match(reply.content, /APPROVED/);
  f.remote.review.state = "DISMISSED";
  f.remote.reviews = [f.remote.review];
  await f.bot.githubEvent(
    job("pull_request_review", {
      action: "dismissed",
      pull_request: f.remote.pr,
      review: { ...f.remote.review, state: "APPROVED" },
    }),
  );
  assert.equal(f.zoo.messages.size, 2);
  assert.match(f.zoo.messages.get(reply.id)!.content, /DISMISSED/);
  assert.match(f.zoo.messages.get(root.message)!.content, /DISMISSED/);
  f.remote.checks[0].conclusion = "failure";
  await f.bot.githubEvent(
    job("check_run", {
      action: "completed",
      check_run: {
        id: 1,
        head_sha: f.remote.pr.head.sha,
        name: "CI",
        conclusion: "failure",
        pull_requests: [{ number: 8 }],
      },
    }),
  );
  assert.equal(
    f.zoo.messages.size,
    2,
    "Checks metadata refresh does not add noise without a checks subscription",
  );
  assert.match(f.zoo.messages.get(root.message)!.content, /CI: failure/);
});

test("workflow and deployment status transitions retain one parent and append progress replies", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  await f.bot.githubEvent(job("workflow_run", { action: "requested", workflow_run: f.remote.run }));
  const root = f.store.thread("channel", { repo: "acme/project", kind: "workflow", number: "42" })!;
  assert.equal(f.zoo.messages.size, 1);
  f.remote.run.status = "in_progress";
  await f.bot.githubEvent(
    job("workflow_run", { action: "in_progress", workflow_run: { ...f.remote.run } }),
  );
  f.remote.run.status = "completed";
  f.remote.run.conclusion = "failure";
  const completed = job("workflow_run", { action: "completed", workflow_run: { ...f.remote.run } });
  await f.bot.githubEvent(completed);
  await f.bot.githubEvent(completed);
  assert.equal(f.zoo.messages.size, 3);
  assert.match(f.zoo.messages.get(root.message)!.content, /failure/);
  assert.equal([...f.zoo.messages.values()].filter((m) => m.parent_id === root.message).length, 2);
  await f.bot.githubEvent(
    job("deployment_status", {
      deployment: f.remote.deployment,
      deployment_status: f.remote.deploymentState,
    }),
  );
  f.remote.deploymentState = { id: 112, state: "success", description: "Ready" };
  await f.bot.githubEvent(
    job("deployment_status", {
      deployment: f.remote.deployment,
      deployment_status: f.remote.deploymentState,
    }),
  );
  const deployed = f.store.thread("channel", {
    repo: "acme/project",
    kind: "deployment",
    number: "11",
  })!;
  assert.match(f.zoo.messages.get(deployed.message)!.content, /success/);
  assert.match(f.zoo.messages.get(deployed.message)!.content, /\/deployments/);
});

test("repository/channel permissions apply to notifications, subscriptions, and link previews", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  await f.bot.zooEvent(command("subscribe acme/project", "other-channel"));
  assert.equal(f.remote.readRequests.length, 0);
  await f.bot.githubEvent(
    job("issues", { action: "opened", issue: f.remote.issue, installation: { id: 999 } }),
  );
  assert.equal(f.zoo.messages.size, 0);
  assert.equal(
    await preview(f.config, f.github, "https://github.com/other/private/issues/7", "channel"),
    undefined,
  );
  assert.equal(
    await preview(f.config, f.github, "https://github.com/acme/project/issues/7", "other-channel"),
    undefined,
  );
  assert.equal(
    await preview(
      f.config,
      f.github,
      "https://github.com@evil.example/acme/project/issues/7",
      "channel",
    ),
    undefined,
  );
  assert.equal(
    await preview(
      f.config,
      f.github,
      "https://github.com/acme/project%2F..%2F..%2Fuser/issues/7",
      "channel",
    ),
    undefined,
  );
  await f.bot.githubEvent(
    job("installation_repositories", { repositories_removed: [{ full_name: "acme/project" }] }),
  );
  await f.bot.githubEvent(job("issues", { action: "opened", issue: f.remote.issue }));
  assert.equal(f.zoo.messages.size, 0);
});

test("one forbidden Zoo destination does not block other permitted channels", async (t) => {
  const f = fixture({
    rules: [
      { repository: "acme/project", installationId: 10, channels: ["channel", "aaa-denied"] },
    ],
  });
  t.after(() => f.store.close());
  f.store.subscribe({ repo: "acme/project", channel: "aaa-denied", features: ["issues"] });
  f.zoo.deny.add("aaa-denied");
  await f.bot.githubEvent(job("issues", { action: "opened", issue: f.remote.issue }));
  assert.equal([...f.zoo.messages.values()].filter((m) => m.channel_id === "channel").length, 1);
});

test("branch, label, workflow, and broadcast filters follow channel subscriptions", () => {
  const tokens = words(
    'acme/project reviews comments:"channel" +label:"priority: HIGH" workflows:{name:"CI build" event:"push","pull_request" branch:"main" actor:"alice"}',
  );
  const sub = subscription("channel", tokens[0], tokens.slice(1));
  assert.equal(sub.label, "priority: HIGH");
  assert.equal(sub.broadcastComments, true);
  assert.deepEqual(sub.workflow?.event, ["push", "pull_request"]);
  const payload = {
    repository: { default_branch: "main" },
    issue: { labels: [{ name: "priority: HIGH" }] },
    workflow_run: {
      name: "CI build",
      event: "push",
      head_branch: "main",
      actor: { login: "alice" },
    },
  };
  assert.equal(permits(sub, "reviews", payload), true);
  assert.equal(permits(sub, "workflows", payload), true);
  assert.equal(permits(sub, "comments", { ...payload, issue: { labels: [] } }), false);
  const commits = subscription("channel", "acme/project", ["commits:users/*"]);
  assert.equal(
    permits(commits, "commits", {
      repository: { default_branch: "main" },
      ref: "refs/heads/users/alice",
    }),
    true,
  );
  assert.equal(
    permits(commits, "commits", { repository: { default_branch: "main" }, ref: "refs/heads/main" }),
    false,
  );
  const filtered = subscription("channel", "acme/project", ["+label:bug"]);
  assert.ok(filtered.features.includes("issues"));
  assert.throws(() => words('workflows:{name:"unterminated}'));
});

test("Organization and repository subscriptions do not duplicate a channel notification", async (t) => {
  const f = fixture({
    rules: [{ repository: "acme/*", installationId: 10, channels: ["channel"] }],
  });
  t.after(() => f.store.close());
  f.store.subscribe({ repo: "acme/*", channel: "channel", features: ["issues", "comments"] });
  await f.bot.githubEvent(job("issues", { action: "opened", issue: f.remote.issue }));
  await f.bot.githubEvent(
    job("issue_comment", { action: "created", issue: f.remote.issue, comment: { id: 99 } }),
  );
  assert.equal(f.zoo.messages.size, 2);
  assert.equal(f.zoo.posts.filter((m) => m.parent_id).length, 1);
});

test("private previews use only configured repositories and code snippets support refs containing slashes", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  const issue = await preview(
    f.config,
    f.github,
    "https://github.com/acme/project/issues/7#issuecomment-99",
    "channel",
  );
  assert.match(issue!.card.description, /Current comment/);
  f.remote.extra = (url) =>
    url.pathname.includes("/contents/")
      ? url.searchParams.get("ref") === "feature/branch"
        ? Response.json({
            type: "file",
            size: 16,
            path: "src/code.ts",
            encoding: "base64",
            content: Buffer.from("one\ntwo\nthree\n").toString("base64"),
          })
        : Response.json({}, { status: 404 })
      : undefined;
  const code = await preview(
    f.config,
    f.github,
    "https://github.com/acme/project/blob/feature/branch/src/code.ts#L2-L3",
    "channel",
  );
  assert.equal(code!.card.description, "two\nthree");
  assert.ok(
    f.remote.readRequests.some((r) => new URL(r.url).searchParams.get("ref") === "feature/branch"),
  );
});

test("review and check pagination includes state beyond the first 100 entries", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.remote.reviews = Array.from({ length: 101 }, (_, i) => ({
    id: i,
    user: { login: "alice" },
    state: i === 100 ? "APPROVED" : "COMMENTED",
  }));
  f.remote.checks = Array.from({ length: 101 }, (_, i) => ({
    id: i,
    name: `Check ${i}`,
    status: "completed",
    conclusion: i === 100 ? "failure" : "success",
  }));
  const snapshot = await f.github.snapshot({ repo: "acme/project", kind: "pull", number: "8" });
  assert.deepEqual(snapshot.reviews, ["alice: APPROVED"]);
  assert.match(snapshot.checkSummary, /Check 100: failure/);
});

test("ETag reads preserve authorization and do not expose mutable cached objects", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  let count = 0;
  f.remote.extra = (url, init) => {
    if (url.pathname !== "/repos/acme/project") return;
    count++;
    if (count === 2) {
      assert.equal(new Headers(init.headers).get("if-none-match"), '"repo-v1"');
      return new Response(null, { status: 304 });
    }
    return Response.json({ full_name: "acme/project" }, { headers: { etag: '"repo-v1"' } });
  };
  const first = await f.github.read("acme/project", "");
  first.leaked = "mutation";
  const second = await f.github.read("acme/project", "");
  assert.equal(second.leaked, undefined);
  assert.equal(count, 2);
});

test("rate limits persist a delayed retry, while permission failures are permanent", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.remote.extra = (url) =>
    url.pathname === "/repos/acme/project"
      ? Response.json({}, { status: 403, headers: { "retry-after": "120" } })
      : undefined;
  let error: GitHubError | undefined;
  try {
    await f.github.read("acme/project", "");
  } catch (e) {
    error = e as GitHubError;
  }
  assert.equal(error!.retryable, true);
  assert.equal(error!.retryAfter, 120000);
  f.store.enqueue("rate", "github", "issues", {});
  const queued = f.store.next()!;
  f.store.fail(queued, error!.code, error!.retryable, error!.retryAfter);
  const row = f.store.db.prepare("SELECT status,available FROM jobs WHERE id='rate'").get()!;
  assert.equal(row.status, "queued");
  assert.ok(Number(row.available) >= Date.now() + 119000);
  assert.equal(new GitHubError(403).retryable, false);
});

test("scheduled channel reminders are timezone aware and post at most once per day", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.remote.pr.requested_reviewers = [{ login: "alice" }];
  f.store.set("reminder:channel", { time: "00:00", zone: "Asia/Tokyo" });
  await f.bot.reminders();
  await f.bot.reminders();
  assert.equal(f.zoo.messages.size, 1);
  assert.match([...f.zoo.messages.values()][0].content, /レビュー依頼リマインダー/);
  assert.match([...f.zoo.messages.values()][0].content, /@alice_zoo/);
});

test("Discussions answer updates edit the parent, and discussion comment edits reuse its reply", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  await f.bot.githubEvent(
    job("discussion", { action: "created", discussion: { ...f.remote.discussion, number: 9 } }),
  );
  const root = f.store.thread("channel", {
    repo: "acme/project",
    kind: "discussion",
    number: "9",
  })!;
  f.remote.discussion.isAnswered = true;
  await f.bot.githubEvent(
    job("discussion", { action: "answered", discussion: { ...f.remote.discussion } }),
  );
  assert.match(f.zoo.messages.get(root.message)!.content, /answered/);
  await f.bot.githubEvent(
    job("discussion_comment", {
      action: "created",
      discussion: { number: 9 },
      comment: { id: 5, node_id: "DC_1", body: "stale" },
    }),
  );
  const comment = [...f.zoo.messages.values()].find((m) =>
    m.content.includes("Discussion comment"),
  )!;
  assert.ok(comment);
  assert.equal(comment.parent_id, root.message);
  f.remote.extra = (url, init) =>
    url.pathname === "/graphql" && String(init.body).includes("DiscussionComment")
      ? Response.json({
          data: {
            node: { id: "DC_1", body: "Updated discussion comment", author: { login: "alice" } },
          },
        })
      : undefined;
  await f.bot.githubEvent(
    job("discussion_comment", {
      action: "edited",
      discussion: { number: 9 },
      comment: { id: 5, node_id: "DC_1" },
    }),
  );
  assert.match(f.zoo.messages.get(comment.id)!.content, /Updated discussion comment/);
  await f.bot.githubEvent(
    job("discussion_comment", {
      action: "deleted",
      discussion: { number: 9 },
      comment: { id: 5, node_id: "DC_1" },
    }),
  );
  assert.equal(f.zoo.messages.has(comment.id), false);
});

test("recreated branches update the same parent and append a distinct state change reply", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  let exists = true;
  f.remote.extra = (url) =>
    url.pathname.startsWith("/repos/acme/project/git/ref/")
      ? exists
        ? Response.json({ object: { sha: "b".repeat(40) } })
        : Response.json({}, { status: 404 })
      : undefined;
  const payload = { ref_type: "branch", ref: "feature" };
  const first = job("create", payload);
  await f.bot.githubEvent(first);
  await f.bot.githubEvent(first);
  assert.equal(
    f.zoo.messages.size,
    1,
    "Retrying initial publication must not add an opening reply",
  );
  exists = false;
  await f.bot.githubEvent(job("delete", payload));
  const root = f.store.thread("channel", {
    repo: "acme/project",
    kind: "branch",
    number: "branch:feature",
  })!;
  assert.match(f.zoo.messages.get(root.message)!.content, /deleted/);
  exists = true;
  await f.bot.githubEvent(job("create", payload));
  assert.equal(f.zoo.messages.size, 3);
  assert.match(f.zoo.messages.get(root.message)!.content, /active/);
});

test("label removal still refreshes a tracked parent without creating an unwanted notification", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.store.subscribe({
    repo: "acme/project",
    channel: "channel",
    features: ["issues"],
    label: "bug",
  });
  await f.bot.githubEvent(job("issues", { action: "opened", issue: f.remote.issue }));
  f.remote.issue.labels = [];
  f.remote.issue.title = "Relabeled";
  await f.bot.githubEvent(job("issues", { action: "unlabeled", issue: f.remote.issue }));
  assert.equal(f.zoo.messages.size, 1);
  const root = [...f.zoo.messages.values()][0];
  assert.match(root.content, /Relabeled/);
  assert.doesNotMatch(root.content, /Labels: bug/);
});

test("losing repository access must not be mistaken for a deleted GitHub comment", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  await f.bot.githubEvent(
    job("issue_comment", { action: "created", issue: f.remote.issue, comment: { id: 99 } }),
  );
  assert.equal(f.zoo.messages.size, 2);
  f.remote.extra = (url) =>
    url.pathname === "/repos/acme/project" || url.pathname.endsWith("/issues/comments/99")
      ? Response.json({}, { status: 404 })
      : undefined;
  await assert.rejects(
    f.bot.githubEvent(
      job("issue_comment", { action: "edited", issue: f.remote.issue, comment: { id: 99 } }),
    ),
    /GitHub API 404/,
  );
  assert.equal(f.zoo.messages.size, 2);
  assert.equal(f.zoo.deleted.length, 0);
});

test("removed wildcard permissions do not block reminders for other allowed channels", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.remote.pr.requested_reviewers = [{ login: "alice" }];
  f.store.subscribe({ repo: "removed/*", channel: "removed-channel", features: ["pulls"] });
  f.store.set("reminder:removed-channel", { time: "00:00", zone: "UTC" });
  f.store.set("reminder:channel", { time: "00:00", zone: "UTC" });
  await f.bot.reminders();
  assert.equal(f.zoo.messages.size, 1);
  assert.equal([...f.zoo.messages.values()][0].channel_id, "channel");
  assert.ok(f.remote.readRequests.every((r) => !String(r.url).includes("removed")));
});
