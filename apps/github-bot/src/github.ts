import { sign } from "node:crypto";
import {
  BotError,
  repoName,
  ruleFor,
  matches,
  type Config,
  type Json,
  type Target,
} from "./types.ts";
import { Store } from "./store.ts";
export class GitHubError extends BotError {
  readonly status: number;
  constructor(status: number, retryAfter = 0) {
    super(
      `GitHub API ${status}`,
      status === 429 || status >= 500 || (status === 403 && retryAfter > 0),
      retryAfter,
    );
    this.status = status;
  }
}
/** Repository access is read-only. The only POSTs are token issuance and GraphQL queries. */
export class GitHub {
  private tokens = new Map<number, { token: string; expires: number }>();
  private readonly stopping = new AbortController();
  private readonly cache = new Map<string, { etag: string; value: any }>();
  readonly config: Config;
  readonly store: Store;
  private readonly fetcher: typeof fetch;
  constructor(config: Config, store: Store, fetcher: typeof fetch = fetch) {
    this.config = config;
    this.store = store;
    this.fetcher = fetcher;
  }
  stop() {
    this.stopping.abort();
  }
  jwt(): string {
    const now = Math.floor(Date.now() / 1000),
      head = Buffer.from(JSON.stringify({ alg: "RS256", typ: "JWT" })).toString("base64url"),
      body = Buffer.from(
        JSON.stringify({ iat: now - 60, exp: now + 540, iss: this.config.appId }),
      ).toString("base64url");
    const input = `${head}.${body}`;
    return `${input}.${sign("RSA-SHA256", Buffer.from(input), this.config.privateKey).toString("base64url")}`;
  }
  async api(path: string, token: string, method = "GET", body?: Json): Promise<any> {
    if (!path.startsWith("/") || path.startsWith("//") || /[\r\n]/.test(path))
      throw new BotError("Invalid GitHub API path");
    const tokenRequest =
      method === "POST" && /^\/app\/installations\/[1-9]\d*\/access_tokens$/.test(path);
    const graphQuery =
      method === "POST" &&
      path === "/graphql" &&
      typeof body?.query === "string" &&
      /^\s*query\b/.test(body.query) &&
      !/\bmutation\b/.test(body.query);
    if (method !== "GET" && !tokenRequest && !graphQuery)
      throw new BotError("GitHub repository writes are disabled");
    const cacheKey = `${token}:${path}`,
      cached = method === "GET" ? this.cache.get(cacheKey) : undefined;
    const response = await this.fetcher(`https://api.github.com${path}`, {
      method,
      headers: {
        accept: "application/vnd.github+json",
        "content-type": "application/json",
        "user-agent": "zoo-github-bot",
        "x-github-api-version": "2026-03-10",
        ...(cached ? { "if-none-match": cached.etag } : {}),
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "error",
      signal: AbortSignal.any([AbortSignal.timeout(15000), this.stopping.signal]),
    });
    if (response.status === 304 && cached) return structuredClone(cached.value);
    if (!response.ok) {
      const retry = Number(response.headers.get("retry-after") ?? 0) * 1000,
        reset =
          response.headers.get("x-ratelimit-remaining") === "0"
            ? Number(response.headers.get("x-ratelimit-reset") ?? 0) * 1000 - Date.now()
            : 0;
      throw new GitHubError(response.status, Math.max(0, retry, reset));
    }
    if (response.status === 204) return {};
    const chunks: Uint8Array[] = [];
    let size = 0;
    if (response.body)
      for await (const chunk of response.body) {
        size += chunk.byteLength;
        if (size > 5 * 1024 * 1024) throw new BotError("GitHub response too large");
        chunks.push(chunk);
      }
    const content = Buffer.concat(chunks).toString("utf8"),
      value = content.trim() ? JSON.parse(content) : {};
    const etag = response.headers.get("etag");
    if (method === "GET" && etag && size <= 200000) {
      this.cache.delete(cacheKey);
      this.cache.set(cacheKey, { etag, value: structuredClone(value) });
      if (this.cache.size > 128) this.cache.delete(this.cache.keys().next().value!);
    }
    return value;
  }
  async installation(repo: string): Promise<string> {
    const rule = ruleFor(this.config, repo);
    if (
      !rule ||
      this.store.get(`installation-disabled:${rule.installationId}`) ||
      this.store.get(`repo-disabled:${repo}`)
    )
      throw new BotError("GitHub installation unavailable");
    const cached = this.tokens.get(rule.installationId);
    if (cached && cached.expires > Date.now() + 60000) return cached.token;
    const result = await this.api(
      `/app/installations/${rule.installationId}/access_tokens`,
      this.jwt(),
      "POST",
    );
    if (typeof result.token !== "string" || !Number.isFinite(Date.parse(result.expires_at)))
      throw new BotError("Invalid installation token response");
    this.tokens.set(rule.installationId, {
      token: result.token,
      expires: Date.parse(result.expires_at),
    });
    return result.token;
  }
  invalidate(installation: number) {
    this.tokens.delete(installation);
  }
  displayName(repo: string): string {
    return this.store.get<string>(`repository-name:${repo.toLowerCase()}`) ?? repo;
  }
  rememberName(name: string) {
    const key = repoName(name);
    this.store.set(`repository-name:${key}`, name);
    const owner = name.split("/")[0];
    this.store.set(`repository-name:${owner.toLowerCase()}/*`, `${owner}/*`);
  }
  async read(repo: string, path: string): Promise<any> {
    for (let attempt = 0; attempt < 2; attempt++)
      try {
        const result = await this.api(
          `/repos/${repoName(repo)}${path}`,
          await this.installation(repo),
        );
        if (
          !path &&
          typeof result.full_name === "string" &&
          result.full_name.toLowerCase() === repo.toLowerCase()
        )
          this.rememberName(result.full_name);
        return result;
      } catch (error) {
        if (attempt === 0 && error instanceof GitHubError && error.status === 401) {
          this.invalidate(ruleFor(this.config, repo)!.installationId);
          continue;
        }
        throw error;
      }
  }
  async pages(repo: string, path: string): Promise<Json[]> {
    const results: Json[] = [];
    for (let page = 1; page <= 20; page++) {
      const rows = await this.read(
        repo,
        `${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`,
      );
      if (!Array.isArray(rows)) throw new BotError("Invalid GitHub page response");
      results.push(...rows);
      if (rows.length < 100) return results;
    }
    throw new BotError("GitHub page limit exceeded");
  }
  async repositories(pattern: string): Promise<string[]> {
    if (!pattern.endsWith("/*")) return [repoName(pattern)];
    const token = await this.installation(pattern),
      repos: string[] = [];
    for (let page = 1; page <= 20; page++) {
      const r = await this.api(`/installation/repositories?per_page=100&page=${page}`, token);
      for (const repository of r.repositories) this.rememberName(repository.full_name);
      repos.push(
        ...r.repositories
          .map((v: Json) => String(v.full_name).toLowerCase())
          .filter((repo: string) => matches(pattern, repo)),
      );
      if (r.repositories.length < 100) return repos;
    }
    throw new BotError("Organization exceeds 2,000 repositories");
  }
  async checkRuns(repo: string, sha: string): Promise<Json[]> {
    const result: Json[] = [];
    for (let page = 1; page <= 20; page++) {
      const r = await this.read(
        repo,
        `/commits/${encodeURIComponent(sha)}/check-runs?per_page=100&page=${page}`,
      );
      if (!Array.isArray(r.check_runs)) throw new BotError("Invalid check run page");
      result.push(...r.check_runs);
      if (r.check_runs.length < 100 || result.length >= r.total_count) return result;
    }
    throw new BotError("GitHub check run page limit exceeded");
  }
  async checks(repo: string, sha: string): Promise<{ state: string; summary: string }> {
    const [status, checks] = await Promise.all([
      this.read(repo, `/commits/${encodeURIComponent(sha)}/status`),
      this.checkRuns(repo, sha),
    ]);
    const runs: Json[] = checks;
    const pending =
      runs.some((r) => r.status !== "completed") ||
      (status.total_count > 0 && status.state === "pending");
    const failed =
      runs.some((r) =>
        ["failure", "cancelled", "timed_out", "action_required", "stale"].includes(r.conclusion),
      ) || ["failure", "error"].includes(status.state);
    return {
      state:
        !runs.length && !status.total_count
          ? "no checks"
          : failed
            ? "failure"
            : pending
              ? "pending"
              : "success",
      summary: [
        ...(status.total_count ? [status.state] : []),
        ...runs.map((c) => `${c.name}: ${c.conclusion ?? c.status}`),
      ].join(" / "),
    };
  }
  async snapshot(target: Target): Promise<Json> {
    const n = encodeURIComponent(target.number);
    if (target.kind === "branch") {
      const cut = target.number.indexOf(":"),
        type = target.number.slice(0, cut),
        ref = target.number.slice(cut + 1);
      try {
        const data = await this.read(
          target.repo,
          `/git/ref/${type === "tag" ? "tags" : "heads"}/${encodeURIComponent(ref)}`,
        );
        return { title: `${type}: ${ref}`, state: "active", body: data.object?.sha ?? "" };
      } catch (error) {
        if (error instanceof GitHubError && error.status === 404) {
          await this.read(target.repo, "");
          return { title: `${type}: ${ref}`, state: "deleted", body: "" };
        }
        throw error;
      }
    }
    if (target.kind === "discussion") {
      const [owner, name] = target.repo.split("/");
      const r = await this.api("/graphql", await this.installation(target.repo), "POST", {
        query: `query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){discussion(number:$number){id number title body url isAnswered author{login} labels(first:100){nodes{name}}}}}`,
        variables: { owner, name, number: Number(target.number) },
      });
      if (r.errors?.length) throw new BotError("GitHub discussion query failed");
      if (!r.data?.repository?.discussion) throw new GitHubError(404);
      return r.data.repository.discussion;
    }
    const path =
      target.kind === "pull"
        ? `/pulls/${n}`
        : target.kind === "issue"
          ? `/issues/${n}`
          : target.kind === "workflow"
            ? `/actions/runs/${n}`
            : target.kind === "deployment"
              ? `/deployments/${n}`
              : target.kind === "release"
                ? `/releases/tags/${n}`
                : target.kind === "commit" || target.kind === "check"
                  ? `/commits/${n}`
                  : "";
    const result = await this.read(target.repo, path);
    if (target.kind === "pull") {
      const [reviews, checks] = await Promise.all([
        this.pages(target.repo, `/pulls/${n}/reviews`),
        this.checks(target.repo, result.head.sha),
      ]);
      const latest = new Map<string, Json>();
      for (const review of reviews) if (review.user?.login) latest.set(review.user.login, review);
      result.reviews = [...latest.values()].map((r) => `${r.user.login}: ${r.state}`);
      result.checkSummary = checks.summary;
    }
    if (target.kind === "check") {
      const checks = await this.checks(target.repo, target.number);
      result.state = checks.state;
      result.checkSummary = checks.summary;
    }
    if (target.kind === "deployment") {
      result.latest = (await this.read(target.repo, `/deployments/${n}/statuses?per_page=1`))[0];
      let run: RegExpExecArray | null = null;
      try {
        const log = new URL(result.latest?.log_url ?? "");
        if (log.protocol === "https:" && log.hostname === "github.com")
          run = new RegExp(
            `^/${target.repo.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/actions/runs/(\\d+)(?:/job/(\\d+))?/?$`,
            "i",
          ).exec(log.pathname);
      } catch {
        /* Deployment is not associated with a workflow. */
      }
      if (run) {
        try {
          const workflow = await this.read(target.repo, `/actions/runs/${run[1]}`);
          const job = run[2] ? await this.read(target.repo, `/actions/jobs/${run[2]}`) : undefined;
          result.deploymentWorkflow = {
            title: `${workflow.name} #${workflow.run_number}${job?.name ? ` / ${job.name}` : ""}`,
            url: job?.html_url ?? workflow.html_url,
          };
        } catch (error) {
          if (!(error instanceof GitHubError) || ![403, 404].includes(error.status)) throw error;
        }
      }
    }
    return result;
  }
}
