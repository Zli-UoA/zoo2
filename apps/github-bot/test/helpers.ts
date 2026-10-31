import { generateKeyPairSync, randomUUID } from "node:crypto";
import { Store } from "../src/store.ts";
import { GitHub } from "../src/github.ts";
import { GitHubBot } from "../src/bot.ts";
import { ZooError, type AppEvent } from "../../../sdk/src/index.ts";
import type { Config, Json, Zoo } from "../src/types.ts";
import type { Job } from "../src/store.ts";
export const pair = generateKeyPairSync("rsa", { modulusLength: 2048 });
export class ZooDouble implements Zoo {
  posts: Json[] = [];
  updates: Json[] = [];
  calls: Json[] = [];
  deleted: string[] = [];
  messages = new Map<string, Json>();
  deny = new Set<string>();
  async request<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
    this.calls.push({ method, path, body });
    if (path === "/auth") return { app_id: "app", bot_user_id: "bot" } as T;
    if (path.includes("/profile")) return { user_name: "alice_zoo" } as T;
    return { id: randomUUID() } as T;
  }
  postMessage: Zoo["postMessage"] = async (input) => {
    if (this.deny.has(input.channel_id))
      throw Object.assign(new Error("forbidden"), { status: 403 });
    const id = input.id ?? randomUUID();
    this.posts.push({ ...input, id });
    const existing = this.messages.get(id);
    if (
      existing &&
      (existing.channel_id !== input.channel_id ||
        existing.content !== input.content ||
        existing.parent_id !== input.parent_id)
    )
      throw new ZooError(409, "Idempotency key already used");
    if (!this.messages.has(id)) this.messages.set(id, { ...input, id });
    return { id };
  };
  updateMessage: Zoo["updateMessage"] = async (id, content) => {
    this.updates.push({ id, content });
    const message = this.messages.get(id);
    if (!message) throw Object.assign(new Error("not found"), { status: 404 });
    message.content = content;
    return { id };
  };
  deleteMessage: Zoo["deleteMessage"] = async (id) => {
    this.deleted.push(id);
    this.messages.delete(id);
    return { id };
  };
  setBlocks: Zoo["setBlocks"] = async (id, blocks) => {
    this.calls.push({ id, blocks });
    return { id };
  };
}
export function fixture(overrides: Partial<Config> = {}, database?: Store) {
  const config: Config = {
    zooUrl: "https://zoo.example",
    rules: [{ repository: "acme/project", channels: ["channel"], installationId: 10 }],
    users: { alice: "zoo-alice" },
    appId: "42",
    privateKey: pair.privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
    githubSecret: "github-test-secret",
    zooSecret: "zoo-test-secret",
    encryptionKey: Buffer.alloc(32, 7),
    database: ":memory:",
    port: 3001,
    host: "127.0.0.1",
    ...overrides,
  };
  const store = database ?? new Store(config.database, config.encryptionKey),
    zoo = new ZooDouble();
  const remote = {
    issue: {
      id: 1,
      number: 7,
      title: "Issue",
      body: "Current issue body",
      state: "open",
      labels: [{ name: "bug" }],
      assignees: [],
      comments: 1,
      updated_at: "2026-10-30T00:00:00Z",
    } as Json,
    pr: {
      id: 2,
      number: 8,
      title: "Pull request",
      body: "Current PR body",
      state: "open",
      draft: false,
      merged: false,
      head: { sha: "a".repeat(40) },
      labels: [],
      assignees: [],
      requested_reviewers: [],
      comments: 0,
      review_comments: 0,
      updated_at: "2026-10-30T00:00:00Z",
    } as Json,
    reviews: [] as Json[],
    review: {
      id: 3,
      user: { login: "alice" },
      state: "APPROVED",
      body: "Looks good",
      submitted_at: "2026-10-30T00:00:00Z",
    } as Json,
    comment: {
      id: 99,
      user: { login: "alice" },
      body: "Current comment",
      updated_at: "2026-10-30T00:00:00Z",
      html_url: "https://github.com/acme/project/issues/7#issuecomment-99",
      issue_url: "https://api.github.com/repos/acme/project/issues/7",
    } as Json | null,
    run: {
      id: 42,
      name: "CI",
      status: "queued",
      conclusion: null,
      event: "pull_request",
      head_branch: "feature",
      pull_requests: [{ base: { ref: "main" } }],
      actor: { login: "alice" },
      run_attempt: 1,
      updated_at: "2026-10-30T00:00:00Z",
    } as Json,
    deployment: { id: 11, environment: "production" } as Json,
    deploymentState: { id: 111, state: "queued", description: "Preparing" } as Json,
    checks: [{ id: 1, name: "CI", status: "completed", conclusion: "success" }] as Json[],
    status: { state: "success", total_count: 0 } as Json,
    discussion: {
      id: "D_1",
      number: 9,
      title: "Discussion",
      body: "Discussion body",
      isAnswered: false,
      labels: { nodes: [] },
    } as Json,
    readRequests: [] as Json[],
    extra: undefined as undefined | ((url: URL, init: RequestInit) => Response | undefined),
  };
  const fetcher: typeof fetch = async (input, options) => {
    const url = new URL(String(input)),
      init = options ?? {},
      method = init.method ?? "GET";
    remote.readRequests.push({
      url: url.toString(),
      method,
      headers: new Headers(init.headers),
      body: init.body,
    });
    const custom = remote.extra?.(url, init);
    if (custom) return custom;
    if (/^\/app\/installations\/\d+\/access_tokens$/.test(url.pathname) && method === "POST")
      return Response.json({
        token: "installation-read-token",
        expires_at: new Date(Date.now() + 3600000).toISOString(),
      });
    if (url.pathname === "/graphql")
      return Response.json({
        data: {
          repository: { discussion: remote.discussion },
          node: {
            id: "DC_1",
            body: "Discussion comment",
            url: "https://github.com/acme/project/discussions/9#discussioncomment-5",
            author: { login: "alice" },
          },
        },
      });
    if (method !== "GET") throw new Error(`Unexpected GitHub write ${method} ${url.pathname}`);
    const page = Number(url.searchParams.get("page") ?? 1),
      p = url.pathname;
    if (p === "/installation/repositories")
      return Response.json({
        repositories: page === 1 ? [{ full_name: "acme/project" }] : [],
        total_count: 1,
      });
    if (p === "/users/acme")
      return Response.json({ login: "acme", name: "Acme", bio: "Public profile" });
    if (p === "/repos/acme/project")
      return Response.json({
        full_name: "acme/project",
        description: "Repository",
        default_branch: "main",
        owner: { login: "acme" },
      });
    if (p === "/repos/acme/project/issues/7") return Response.json(remote.issue);
    if (p === "/repos/acme/project/pulls/8") return Response.json(remote.pr);
    if (p === "/repos/acme/project/pulls/8/reviews")
      return Response.json(remote.reviews.slice((page - 1) * 100, page * 100));
    if (p === "/repos/acme/project/pulls/8/reviews/3") return Response.json(remote.review);
    if (p.endsWith("/issues/comments/99") || p.endsWith("/pulls/comments/99"))
      return remote.comment
        ? Response.json(remote.comment)
        : Response.json({ message: "Not found" }, { status: 404 });
    if (p.endsWith("/status")) return Response.json(remote.status);
    if (p.endsWith("/check-runs"))
      return Response.json({
        check_runs: remote.checks.slice((page - 1) * 100, page * 100),
        total_count: remote.checks.length,
      });
    if (/\/commits\/[^/]+\/pulls$/.test(p)) return Response.json([{ number: 8 }]);
    if (p === "/repos/acme/project/actions/runs/42") return Response.json(remote.run);
    if (p === "/repos/acme/project/deployments/11") return Response.json(remote.deployment);
    if (p === "/repos/acme/project/deployments/11/statuses")
      return Response.json([remote.deploymentState]);
    if (p === "/repos/acme/project/pulls") return Response.json(page === 1 ? [remote.pr] : []);
    if (/\/commits\/[a-f0-9]+$/.test(p))
      return Response.json({ commit: { message: "Commit message" }, author: { login: "alice" } });
    if (p.startsWith("/repos/acme/project/releases/tags/"))
      return Response.json({ tag_name: "v1", name: "Release", body: "Release notes" });
    if (p.startsWith("/repos/acme/project/contents/"))
      return Response.json({
        type: "file",
        size: 12,
        path: "src/code.ts",
        encoding: "base64",
        content: Buffer.from("one\ntwo\nthree\n").toString("base64"),
      });
    return Response.json({ message: `Unhandled test route ${p}` }, { status: 404 });
  };
  const github = new GitHub(config, store, fetcher),
    bot = new GitHubBot(config, store, github, zoo);
  store.subscribe({
    channel: "channel",
    repo: "acme/project",
    features: [
      "issues",
      "pulls",
      "comments",
      "reviews",
      "checks",
      "deployments",
      "releases",
      "discussions",
      "branches",
      "tags",
      "commits",
      "workflows",
    ],
    workflow: { event: ["pull_request", "push"] },
  });
  return { config, store, zoo, github, bot, remote, fetcher };
}
export function job(event: string, payload: Json, delivery = randomUUID()): Job {
  return {
    id: `github:${delivery}`,
    source: "github",
    event,
    payload: {
      installation: { id: 10 },
      repository: { full_name: "acme/project", default_branch: "main" },
      sender: { login: "alice" },
      ...payload,
    },
    attempts: 1,
    created: Date.now(),
  };
}
export function command(text: string, channel = "channel"): AppEvent {
  return {
    id: randomUUID(),
    version: 1,
    type: "command.invoked",
    actor_id: "zoo-alice",
    channel_id: channel,
    data: { name: "github", values: { text } },
  };
}
