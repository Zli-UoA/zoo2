import { test } from "node:test";
import assert from "node:assert/strict";
import { fixture, job, command } from "./helpers.ts";
import { preview } from "../src/previews.ts";

test("deployment cards show canonical repository spelling, status, commit and workflow links", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.remote.deployment = { id: 11, environment: "production", sha: "a".repeat(40), ref: "Main" };
  f.remote.deploymentState = {
    id: 112,
    state: "success",
    created_at: "2026-10-31T10:00:00Z",
    environment_url: "https://zli.chat",
    log_url: "https://github.com/Acme/Project/actions/runs/42/job/123",
  };
  f.remote.run.run_number = 68;
  f.remote.run.html_url = "https://github.com/Acme/Project/actions/runs/42";
  f.remote.extra = (url) =>
    url.pathname === "/repos/acme/project/actions/jobs/123"
      ? Response.json({
          name: "deploy",
          html_url: "https://github.com/Acme/Project/actions/runs/42/job/123",
        })
      : undefined;
  await f.bot.githubEvent(
    job("deployment_status", {
      repository: { full_name: "Acme/Project" },
      sender: { login: "Alice" },
      deployment: f.remote.deployment,
      deployment_status: f.remote.deploymentState,
    }),
  );
  const message = [...f.zoo.messages.values()][0];
  assert.match(message.blocks[0].text, /by Alice/);
  const card = message.blocks[1].card;
  assert.equal(card.source, "Acme/Project");
  assert.equal(card.color, "#2da44e");
  assert.equal(card.fields.find((f: any) => f.title === "Status").value, "Completed");
  assert.match(card.fields.find((f: any) => f.title === "Commit").url, /Acme\/Project\/commit/);
  assert.equal(card.fields.find((f: any) => f.title === "Workflow").value, "CI #68 / deploy");
});

test("mixed-case deliveries edit the same PR card and comment, preserving canonical display names", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  const repository = { full_name: "Acme/Project" };
  await f.bot.githubEvent(
    job("pull_request", { action: "opened", repository, pull_request: f.remote.pr }),
  );
  const root = [...f.zoo.messages.values()][0];
  f.remote.pr.merged = true;
  f.remote.pr.state = "closed";
  await f.bot.githubEvent(
    job("pull_request", { action: "closed", repository, pull_request: f.remote.pr }),
  );
  assert.equal(root.blocks[1].card.color, "#8250df");
  assert.match(root.blocks[1].card.url, /Acme\/Project\/pull\/8/);
  assert.match(root.content, /Acme\/Project/);
  const comment = {
    action: "created",
    repository,
    issue: { ...f.remote.pr, pull_request: {} },
    comment: { id: 99 },
  };
  await f.bot.githubEvent(job("issue_comment", comment));
  const reply = [...f.zoo.messages.values()].find((m) =>
    m.blocks?.[1]?.card.description.includes("Current comment"),
  )!;
  f.remote.comment!.body = "Edited comment";
  await f.bot.githubEvent(job("issue_comment", { ...comment, action: "edited" }));
  assert.equal(f.zoo.messages.get(reply.id)!.blocks[1].card.description, "Edited comment");
  assert.equal(f.store.subscriptions()[0].repo, "acme/project");
  assert.equal(f.zoo.posts.filter((m) => m.id === root.id).length, 1);
});

test("link previews use the canonical name from GitHub metadata", async (t) => {
  const f = fixture();
  t.after(() => f.store.close());
  f.remote.extra = (url) =>
    url.pathname === "/repos/acme/project"
      ? Response.json({ full_name: "Acme/Project", description: "Repo", owner: { login: "Acme" } })
      : undefined;
  const card = await preview(
    f.config,
    f.github,
    "https://github.com/ACME/PROJECT/issues/7",
    "channel",
  );
  assert.match(card!.card.title, /^Acme\/Project #7/);
  assert.match(card!.card.url, /github\.com\/Acme\/Project\/issues\/7/);
  await f.bot.zooEvent(command("subscribe ACME/PROJECT"));
  const response = f.zoo.calls.findLast((c) => c.path === "/surfaces")!;
  assert.match(response.body.blocks[0].text, /^Acme\/Project:/);
});
