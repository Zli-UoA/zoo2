import { ZooError, type AppEvent } from "../../../sdk/src/index.ts";
import { Store, type Job } from "./store.ts";
import { GitHub, GitHubError } from "./github.ts";
import { notices, summary, type Notice } from "./events.ts";
import { subscription, permits, words } from "./subscriptions.ts";
import { preview } from "./previews.ts";
import {
  BotError,
  actor,
  id,
  matches,
  repoName,
  resource,
  ruleFor,
  text,
  type Config,
  type Json,
  type Thread,
  type Zoo,
} from "./types.ts";
const HELP = `GitHub 通知 Bot\n/github subscribe owner/repo [issues pulls commits releases deployments workflows reviews comments branches tags discussions checks]\n/github unsubscribe owner/repo [features]\n/github subscribe owner（許可された Organization 全体）\n/github subscribe list features\n/github settings threading on|off\n/github reminders HH:MM Asia/Tokyo | off\nGitHub 上のコメント・状態変更を Zoo に通知します。GitHub の変更操作は GitHub 上で行ってください。`;
export class GitHubBot {
  readonly config: Config;
  readonly store: Store;
  readonly github: GitHub;
  readonly zoo: Zoo;
  private busy = false;
  private stopping = false;
  private lastMaintenance = 0;
  constructor(config: Config, store: Store, github: GitHub, zoo: Zoo) {
    this.config = config;
    this.store = store;
    this.github = github;
    this.zoo = zoo;
  }
  async stop() {
    this.stopping = true;
    this.github.stop();
    while (this.busy) await new Promise((resolve) => setTimeout(resolve, 20));
  }
  allowed(repo: string, channel: string) {
    if (!ruleFor(this.config, repo, channel) || this.store.get(`repo-disabled:${repo}`))
      throw new BotError(
        "このリポジトリからこのチャンネルへの通知は許可されていません。管理者に設定を確認してください。",
      );
  }
  async private(actor: string, channel: string, message: string) {
    return this.zoo.request<Json>("POST", "/surfaces", {
      kind: "ephemeral",
      user_id: actor,
      channel_id: channel,
      blocks: [{ type: "text", text: text(message, 3900) }],
    });
  }
  async publishMessage(
    input: Parameters<Zoo["postMessage"]>[0] & { id: string },
    published: boolean,
  ) {
    if (published) {
      await this.zoo.updateMessage(input.id, input.content);
      return;
    }
    try {
      await this.zoo.postMessage(input);
    } catch (error) {
      // The POST may have succeeded before a crash or a lost response. Our deterministic
      // UUID still identifies the same message, even if GitHub's current body has changed.
      if (error instanceof ZooError && error.status === 409)
        await this.zoo.updateMessage(input.id, input.content);
      else throw error;
    }
  }
  async tick() {
    if (this.busy || this.stopping) return;
    this.busy = true;
    try {
      for (let i = 0; i < 10 && !this.stopping; i++) {
        const job = this.store.next();
        if (!job) break;
        try {
          if (job.source === "github") await this.githubEvent(job);
          else await this.zooEvent(job.payload as unknown as AppEvent);
          this.store.done(job);
        } catch (error) {
          const code = error instanceof BotError ? error.code : "request_failed";
          this.store.fail(
            job,
            code,
            error instanceof BotError ? error.retryable : true,
            error instanceof BotError ? error.retryAfter : 0,
          );
          console.error(JSON.stringify({ job: job.id, event: job.event, error: code }));
        }
      }
      if (
        !this.stopping &&
        !this.store.get("zoo-disabled") &&
        Date.now() - this.lastMaintenance > 60000
      ) {
        this.lastMaintenance = Date.now();
        await this.reminders();
        this.store.cleanup();
      }
    } finally {
      this.busy = false;
    }
  }
  async githubEvent(job: Job) {
    const p = job.payload,
      known = this.config.rules.some((r) => r.installationId === Number(p.installation?.id));
    if (job.event === "installation") {
      if (!known) return;
      const installation = Number(p.installation.id);
      if (["deleted", "suspend"].includes(p.action))
        this.store.set(`installation-disabled:${installation}`, true);
      if (["created", "unsuspend"].includes(p.action))
        this.store.remove(`installation-disabled:${installation}`);
      this.github.invalidate(installation);
      return;
    }
    if (job.event === "installation_repositories") {
      if (!known) return;
      for (const r of p.repositories_removed ?? [])
        this.store.set(`repo-disabled:${String(r.full_name).toLowerCase()}`, true);
      for (const r of p.repositories_added ?? [])
        this.store.remove(`repo-disabled:${String(r.full_name).toLowerCase()}`);
      this.github.invalidate(Number(p.installation.id));
      return;
    }
    if (job.event === "ping" || this.store.get("zoo-disabled")) return;
    const repo = repoName(String(p.repository?.full_name ?? "")),
      rule = ruleFor(this.config, repo);
    if (
      !rule ||
      Number(p.installation?.id) !== rule.installationId ||
      this.store.get(`repo-disabled:${repo}`) ||
      this.store.get(`installation-disabled:${rule.installationId}`)
    )
      return;
    const subscriptions = this.store
      .subscriptions()
      .filter((s) => matches(s.repo, repo) && ruleFor(this.config, repo, s.channel));
    if (!subscriptions.length) return;
    let list = notices(job.event, p, repo);
    if (
      ["status", "check_run", "check_suite"].includes(job.event) &&
      list.some((n) => n.target.kind === "check")
    ) {
      const sha = String(p.sha ?? p.check_run?.head_sha ?? p.check_suite?.head_sha ?? "");
      const pulls = await this.github.read(
        repo,
        `/commits/${encodeURIComponent(sha)}/pulls?per_page=100`,
      );
      list = [
        ...list,
        ...pulls.map(
          (pr: Json) =>
            ({
              target: { repo, kind: "pull", number: String(pr.number) },
              feature: "checks",
              action: p.state,
              body: `${text(p.context ?? p.check_run?.name ?? "Checks")} · ${text(p.state ?? p.check_run?.conclusion ?? p.check_suite?.conclusion)}`,
              mentions: [],
            }) as Notice,
        ),
      ];
    }
    for (const notice of list)
      if (["branches", "tags"].includes(notice.feature)) {
        notice.child = `ref:${job.id}`;
        notice.stateChange = true;
      }
    const snapshots = new Map<string, Json>();
    for (const notice of list) {
      const groups = new Map<string, typeof subscriptions>();
      for (const sub of subscriptions)
        groups.set(sub.channel, [...(groups.get(sub.channel) ?? []), sub]);
      if (
        [...groups.values()].some((group) => group.some((sub) => permits(sub, notice.feature, p)))
      )
        await this.hydrate(job.event, p, notice);
      for (const group of groups.values()) {
        const active = group.filter((sub) => permits(sub, notice.feature, p));
        const sub = {
          ...group[0],
          features: [...new Set(group.flatMap((s) => s.features))],
          broadcastComments: active.some((s) => s.broadcastComments),
          broadcastReviews: active.some((s) => s.broadcastReviews),
        };
        const existing = this.store.thread(sub.channel, notice.target);
        const parentFeature =
          notice.target.kind === "pull"
            ? "pulls"
            : notice.target.kind === "issue"
              ? "issues"
              : "discussions";
        const refreshOnly =
          Boolean(existing) && sub.features.includes(parentFeature) && !active.length;
        if ((!refreshOnly && !active.length) || (notice.deleteChild && !existing)) continue;
        const key = `${notice.target.kind}:${notice.target.number}`;
        const snapshot: Json =
          notice.snapshot ?? snapshots.get(key) ?? (await this.current(notice, existing, p));
        snapshots.set(key, snapshot);

        try {
          const threading = this.store.get<boolean>(`threading:${sub.channel}`) !== false;
          const thread: Thread = {
            ...notice.target,
            channel: sub.channel,
            message:
              existing?.message ??
              id(`thread:${sub.channel}:${repo}:${notice.target.kind}:${notice.target.number}`),
            snapshot,
          };
          this.store.saveThread(thread);
          const published = this.store.get<boolean>(`root-published:${thread.message}`);
          if (!published && !this.store.get(`root-opening-job:${thread.message}`))
            this.store.set(`root-opening-job:${thread.message}`, job.id);
          const root =
            summary(notice.target, snapshot) +
            (published ? "" : `\n${await this.mentions(notice.mentions)}`);
          await this.publishMessage(
            {
              id: thread.message,
              channel_id: thread.channel,
              content: root,
            },
            Boolean(published),
          );
          this.store.set(`root-published:${thread.message}`, true);
          // Opening creates the parent; subsequent state changes and comments become replies.
          const childSource = [
            "issue_comment",
            "pull_request_review",
            "pull_request_review_comment",
            "pull_request_review_thread",
            "discussion_comment",
          ].includes(job.event);
          const initial =
            this.store.get(`root-opening-job:${thread.message}`) === job.id && !childSource;
          const shouldReply =
            !refreshOnly && notice.child && !initial && (notice.action !== "edited" || childSource);
          if (shouldReply) {
            const childKey = `child:${sub.channel}:${repo}:${notice.child}`,
              child = id(childKey),
              known = this.store.get<boolean>(childKey);
            if (notice.deleteChild) {
              if (known) {
                try {
                  await this.zoo.deleteMessage(child);
                } catch (error) {
                  if (![400, 404].includes(Number((error as { status?: number }).status)))
                    throw error;
                }
                this.store.remove(childKey);
              }
              continue;
            }
            const mentions = await this.mentions(notice.mentions);
            // Verified Zoo mentions are appended after untrusted GitHub text is escaped.
            const withMentions = `${text(notice.body, 3400)}${mentions ? `\n${mentions}` : ""}`;
            await this.publishMessage(
              {
                id: child,
                channel_id: sub.channel,
                content: withMentions,
                ...(threading
                  ? {
                      parent_id: thread.message,
                      broadcast: Boolean(
                        notice.stateChange ||
                        (notice.feature === "comments" && sub.broadcastComments) ||
                        (notice.feature === "reviews" && sub.broadcastReviews),
                      ),
                    }
                  : {}),
              },
              Boolean(known),
            );
            this.store.set(childKey, true);
          }
        } catch (error) {
          if ([403, 404].includes(Number((error as { status?: number }).status))) continue;
          throw error;
        }
      }
    }
  }
  async current(notice: Notice, existing: Thread | undefined, payload: Json): Promise<Json> {
    try {
      return await this.github.snapshot(notice.target);
    } catch (error) {
      if (
        error instanceof GitHubError &&
        error.status === 404 &&
        ["deleted", "unpublished"].includes(notice.action)
      ) {
        await this.github.read(notice.target.repo, "");
        return {
          ...(existing?.snapshot ?? payload.release ?? payload.issue ?? payload.pull_request ?? {}),
          state: notice.action,
        } as Json;
      }
      throw error;
    }
  }
  async hydrate(event: string, p: Json, notice: Notice) {
    const repo = notice.target.repo,
      n = encodeURIComponent(notice.target.number);
    if (
      ![
        "issue_comment",
        "pull_request_review_comment",
        "pull_request_review",
        "discussion_comment",
      ].includes(event)
    )
      return;
    if (p.action === "deleted") {
      notice.deleteChild = true;
      return;
    }
    let current: Json;
    try {
      if (event === "discussion_comment") {
        const r = await this.github.api("/graphql", await this.github.installation(repo), "POST", {
          query: `query($id:ID!){node(id:$id){... on DiscussionComment{id body url updatedAt author{login}}}}`,
          variables: { id: p.comment.node_id },
        });
        if (r.errors?.length) throw new BotError("GitHub comment query failed");
        if (!r.data?.node) throw new GitHubError(404);
        current = r.data.node;
      } else
        current = await this.github.read(
          repo,
          event === "issue_comment"
            ? `/issues/comments/${p.comment.id}`
            : event === "pull_request_review_comment"
              ? `/pulls/comments/${p.comment.id}`
              : `/pulls/${n}/reviews/${p.review.id}`,
        );
    } catch (error) {
      if (error instanceof GitHubError && error.status === 404) {
        await this.github.read(repo, "");
        notice.deleteChild = true;
        return;
      }
      throw error;
    }
    const author = current.user?.login ?? current.author?.login ?? "GitHub";
    notice.body = `${text(author)} · ${event === "pull_request_review" ? `review ${text(current.state)}` : "comment"} · ${text(current.updated_at ?? current.updatedAt ?? current.submitted_at, 40)}\n${text(current.body)}`;
    const link = String(current.html_url ?? current.url ?? "");
    try {
      const u = new URL(link);
      if (
        u.protocol === "https:" &&
        u.hostname === "github.com" &&
        u.pathname.toLowerCase().startsWith(`/${repo}/`)
      )
        notice.body += `\n${u}`;
    } catch {
      /* No canonical comment URL available. */
    }
    notice.mentions = [...String(current.body ?? "").matchAll(/@([a-z\d-]+)/gi)].map((m) => m[1]);
  }
  async mentions(logins: string[]): Promise<string> {
    const mentions: string[] = [];
    for (const login of [...new Set(logins)].slice(0, 20)) {
      const actor = this.config.users[login.toLowerCase()];
      if (!actor) continue;
      try {
        const profile = await this.zoo.request<Json>(
          "GET",
          `/users/${encodeURIComponent(actor)}/profile`,
        );
        if (typeof profile.user_name === "string") mentions.push(`@${profile.user_name}`);
      } catch {
        /* Removed or invisible accounts are not mentioned. */
      }
    }
    return mentions.join(" ");
  }
  async zooEvent(event: AppEvent) {
    if (["app.uninstalled", "tokens.revoked"].includes(event.type)) {
      try {
        await this.zoo.request("GET", "/auth");
      } catch {
        this.store.set("zoo-disabled", true);
      }
      return;
    }
    if (
      this.store.get("zoo-disabled") ||
      event.actor_id === this.store.get<Json>("zoo-identity")?.bot_user_id
    )
      return;
    if (!event.actor_id || !event.channel_id) return;
    const user = event.actor_id,
      channel = event.channel_id;
    try {
      if (event.type === "command.invoked") {
        if (event.data.name === "github") await this.command(event);
        return;
      }
      if (event.type === "link.shared") {
        const links = event.data.links;
        if (!Array.isArray(links)) return;
        for (const url of [
          ...new Set(links.filter((u): u is string => typeof u === "string")),
        ].slice(0, 3)) {
          const card = await preview(this.config, this.github, url, channel);
          if (card && typeof event.data.id === "string")
            await this.zoo.request(
              "POST",
              `/messages/${encodeURIComponent(event.data.id)}/unfurls`,
              card,
            );
        }
      }
      // Normal Zoo messages and reactions never call GitHub APIs.
    } catch (error) {
      if (error instanceof GitHubError && error.retryable) throw error;
      if (error instanceof ZooError && (error.status === 429 || error.status >= 500)) throw error;
      if (!(error instanceof BotError) && !Number.isInteger((error as { status?: number }).status))
        throw error;
      const message =
        error instanceof GitHubError
          ? `GitHub の読み取りが拒否されました (${error.status})。GitHub App の権限・インストール先を確認してください。`
          : error instanceof BotError
            ? error.code
            : "Zoo の操作権限・App の参加状態を確認してください。";
      await this.private(user, channel, message);
    }
  }
  async command(event: AppEvent) {
    const user = actor(event),
      channel = event.channel_id!,
      args = words(String((event.data.values as Json)?.text ?? "")),
      command = args.shift() ?? "help";
    if (command === "help") {
      await this.private(user, channel, HELP);
      return;
    }
    if ((command === "subscribe" && args[0] === "list") || command === "subscriptions") {
      const lines = this.store
        .subscriptions(channel)
        .filter((s) => ruleFor(this.config, s.repo, channel))
        .map(
          (s) =>
            `${s.repo}: ${s.features.join(", ")}${s.branch ? ` commits:${s.branch}` : ""}${s.label ? ` +label:${s.label}` : ""}${s.workflow ? ` workflows:${JSON.stringify(s.workflow)}` : ""}`,
        );
      await this.private(user, channel, lines.join("\n") || "このチャンネルには購読がありません。");
      return;
    }
    if (command === "subscribe" || command === "unsubscribe") {
      const value = args.shift();
      if (!value) throw new BotError("owner/repo を指定してください。");
      const repo = repoName(value.includes("/") ? value : `${value}/*`);
      this.allowed(repo, channel);
      if (command === "unsubscribe" && !this.store.subscription(channel, repo)) {
        await this.private(user, channel, `${repo} は購読されていません。`);
        return;
      }
      if (command === "unsubscribe" && !args.length) {
        this.store.unsubscribe(channel, repo);
        await this.private(user, channel, `${repo} の購読を解除しました。`);
        return;
      }
      if (repo.endsWith("/*")) {
        if (!(await this.github.repositories(repo)).length)
          throw new BotError("GitHub App が読み取れるリポジトリがありません。");
      } else await this.github.read(repo, "");
      const sub = subscription(
        channel,
        repo,
        args,
        this.store.subscription(channel, repo),
        command === "unsubscribe",
      );
      this.store.subscribe(sub);
      await this.private(user, channel, `${repo}: ${sub.features.join(", ") || "通知なし"}`);
      return;
    }
    if (command === "settings") {
      if (!this.config.rules.some((r) => r.channels.includes(channel)))
        throw new BotError("このチャンネルは通知先に設定されていません。");
      if (args[0] !== "threading" || !["on", "off"].includes(args[1]))
        throw new BotError("settings threading on|off と指定してください。");
      const enabled = args[1] === "on";
      this.store.set(`threading:${channel}`, enabled);
      await this.private(
        user,
        channel,
        `通知のスレッド表示を${enabled ? "有効" : "無効"}にしました。`,
      );
      return;
    }
    if (command === "reminders") {
      if (args[0] === "off") {
        this.store.remove(`reminder:${channel}`);
        await this.private(
          user,
          channel,
          "このチャンネルのレビュー依頼リマインダーを停止しました。",
        );
        return;
      }
      if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(args[0] ?? "") || !args[1])
        throw new BotError("reminders HH:MM Asia/Tokyo の形式で指定してください。");
      try {
        new Intl.DateTimeFormat("en-US", { timeZone: args[1] }).format();
      } catch {
        throw new BotError("IANA タイムゾーンを指定してください。");
      }
      if (!this.store.subscriptions(channel).length)
        throw new BotError("先にこのチャンネルでリポジトリを購読してください。");
      this.store.set(`reminder:${channel}`, { time: args[0], zone: args[1] });
      await this.private(
        user,
        channel,
        `毎日 ${args[0]} (${args[1]}) に、購読先の未対応レビュー依頼を通知します。`,
      );
      return;
    }
    throw new BotError(HELP);
  }
  async reminders() {
    const rows = this.store.db
      .prepare("SELECT key,value FROM kv WHERE key LIKE 'reminder:%'")
      .all();
    for (const row of rows) {
      const channel = String(row.key).slice(9),
        config = JSON.parse(String(row.value));
      try {
        const now = new Intl.DateTimeFormat("sv-SE", {
          timeZone: config.zone,
          year: "numeric",
          month: "2-digit",
          day: "2-digit",
          hour: "2-digit",
          minute: "2-digit",
          hourCycle: "h23",
        }).format(new Date());
        const day = now.slice(0, 10);
        if (now.slice(-5) < config.time || this.store.get(`reminder-sent:${channel}`) === day)
          continue;
        const lines: string[] = [];
        const seen = new Set<string>();
        for (const sub of this.store
          .subscriptions(channel)
          .filter((s) => ruleFor(this.config, s.repo, channel)))
          for (const repo of await this.github.repositories(sub.repo)) {
            if (this.stopping) return;
            if (!ruleFor(this.config, repo, channel) || seen.has(repo)) continue;
            seen.add(repo);
            const pulls = await this.github.pages(repo, "/pulls?state=open");
            for (const pr of pulls.filter((p: Json) => p.requested_reviewers?.length))
              lines.push(
                `${repo} #${pr.number} ${text(pr.title, 160)}\n${resource({ repo, kind: "pull", number: String(pr.number) })} ${await this.mentions(pr.requested_reviewers.map((r: Json) => r.login))}`,
              );
          }
        if (lines.length)
          await this.zoo.postMessage({
            id: id(`reminder:${channel}:${day}`),
            channel_id: channel,
            content: `レビュー依頼リマインダー (${config.zone})\n${Array.from(lines.join("\n")).slice(0, 3500).join("")}`,
          });
        this.store.set(`reminder-sent:${channel}`, day);
      } catch (error) {
        console.error(
          JSON.stringify({
            task: "reminder",
            channel,
            error: error instanceof BotError ? error.code : "request_failed",
          }),
        );
      }
    }
  }
}
