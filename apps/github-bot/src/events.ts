import { text, resource, type Json, type Target } from "./types.ts";
export interface Notice {
  target: Target;
  feature: string;
  action: string;
  body: string;
  child?: string;
  deleteChild?: boolean;
  stateChange?: boolean;
  snapshot?: Json;
  mentions: string[];
}
export const GITHUB_EVENTS = [
  "ping",
  "installation",
  "installation_repositories",
  "issues",
  "issue_comment",
  "pull_request",
  "pull_request_review",
  "pull_request_review_comment",
  "pull_request_review_thread",
  "push",
  "release",
  "create",
  "delete",
  "discussion",
  "discussion_comment",
  "workflow_run",
  "deployment_status",
  "check_run",
  "check_suite",
  "status",
];
export function notices(event: string, p: Json, repo: string): Notice[] {
  const actor = p.sender?.login ?? "GitHub",
    action = p.action ?? p.deployment_status?.state ?? p.state ?? "updated";
  const displayAction =
    event === "pull_request" && action === "closed" && p.pull_request?.merged ? "merged" : action;
  const changedAt =
    p.comment?.updated_at ??
    p.review?.submitted_at ??
    p.issue?.updated_at ??
    p.pull_request?.updated_at ??
    p.workflow_run?.updated_at;
  const base = {
    action,
    body: `${text(actor)} · ${text(displayAction.replaceAll("_", " "), 200)}${changedAt ? ` · ${text(changedAt, 40)}` : ""}`,
    mentions: [] as string[],
  };
  const target = (kind: Target["kind"], number: unknown) => ({
    repo,
    kind,
    number: String(number),
  });
  const issue = p.issue ?? p.pull_request,
    number = issue?.number ?? p.number;
  if (event === "issues" || event === "pull_request") {
    const mentions = [
      ...(issue.assignees ?? []).map((u: Json) => u.login),
      ...(issue.requested_reviewers ?? []).map((u: Json) => u.login),
      ...String(issue.body ?? "").matchAll(/@([a-z\d-]+)/gi),
    ].map((v: any) => (Array.isArray(v) ? v[1] : v));
    return [
      {
        ...base,
        target: target(event === "issues" ? "issue" : "pull", number),
        feature: event === "issues" ? "issues" : "pulls",
        child: `${event}:${issue.id}:${action}:${issue.updated_at ?? ""}:${p.label?.id ?? ""}:${p.assignee?.id ?? ""}:${p.requested_reviewer?.id ?? ""}`,
        stateChange: ["closed", "reopened"].includes(action),
        mentions,
      },
    ];
  }
  if (
    [
      "issue_comment",
      "pull_request_review",
      "pull_request_review_comment",
      "pull_request_review_thread",
    ].includes(event)
  ) {
    const isPull = event !== "issue_comment" || Boolean(p.issue?.pull_request),
      object = p.comment ?? p.review ?? p.thread;
    return [
      {
        ...base,
        target: target(isPull ? "pull" : "issue", number),
        feature: event === "issue_comment" ? "comments" : "reviews",
        body: `${base.body}${object?.state ? ` · ${text(object.state)}` : ""}\n${text(object?.body)}`,
        child: `${event}:${object?.id}`,
        deleteChild: action === "deleted",
        mentions: [...String(object?.body ?? "").matchAll(/@([a-z\d-]+)/gi)].map((m) => m[1]),
      },
    ];
  }
  if (event === "discussion" || event === "discussion_comment")
    return [
      {
        ...base,
        target: target("discussion", p.discussion.number),
        feature: "discussions",
        body: `${base.body}\n${text(p.comment?.body)}`,
        child: `${event}:${p.comment?.id ?? p.discussion.id}:${event === "discussion" ? action : ""}`,
        deleteChild: action === "deleted" && event === "discussion_comment",
        mentions: [
          ...String(p.comment?.body ?? p.discussion.body ?? "").matchAll(/@([a-z\d-]+)/gi),
        ].map((m) => m[1]),
      },
    ];
  if (event === "workflow_run")
    return [
      {
        ...base,
        target: target("workflow", p.workflow_run.id),
        feature: "workflows",
        body: `${base.body} · ${text(p.workflow_run.conclusion ?? p.workflow_run.status)}`,
        child: `workflow:${p.workflow_run.id}:${p.workflow_run.run_attempt}:${p.workflow_run.status}:${p.workflow_run.conclusion}`,
        mentions: [],
      },
    ];
  if (event === "deployment_status")
    return [
      {
        ...base,
        target: target("deployment", p.deployment.id),
        feature: "deployments",
        body: `${base.body} · ${text(p.deployment.environment)}\n${text(p.deployment_status.description)}`,
        child: `deployment:${p.deployment_status.id}`,
        mentions: [],
      },
    ];
  if (event === "release" && p.release.draft) return [];
  if (event === "release")
    return [
      {
        ...base,
        target: target("release", p.release.tag_name),
        feature: "releases",
        body: `${base.body}\n${text(p.release.body)}`,
        child: `release:${p.release.id}:${action}:${p.release.published_at ?? p.release.created_at}`,
        mentions: [],
      },
    ];
  if (event === "push" && !p.deleted)
    return [
      {
        ...base,
        target: target("commit", p.after),
        feature: "commits",
        body: `${text(p.ref)}\n${(p.commits ?? [])
          .slice(-10)
          .map((c: Json) => `${String(c.id).slice(0, 7)} ${text(c.message, 180)}`)
          .join("\n")}`,
        snapshot: {
          title: `${p.size ?? p.commits?.length ?? 0} commits · ${p.ref}`,
          state: "pushed",
          body: (p.commits ?? [])
            .slice(-10)
            .map((c: Json) => `${String(c.id).slice(0, 7)} ${text(c.message, 180)}`)
            .join("\n"),
        },
        mentions: [],
      },
    ];
  if (["create", "delete"].includes(event) && ["branch", "tag"].includes(p.ref_type))
    return [
      {
        ...base,
        action: event,
        target: target("branch", `${p.ref_type}:${p.ref}`),
        feature: p.ref_type === "branch" ? "branches" : "tags",
        body: `${text(p.ref_type)} ${text(p.ref)} · ${event}`,
        mentions: [],
      },
    ];
  if (["check_run", "check_suite", "status"].includes(event)) {
    const check = p.check_run ?? p.check_suite ?? p;
    if (check.pull_requests?.length)
      return check.pull_requests.map((pr: Json) => ({
        ...base,
        target: target("pull", pr.number),
        feature: "checks",
        body: `${text(check.name ?? "Checks")} · ${text(check.conclusion ?? check.status ?? check.state)}`,
        mentions: [],
      }));
    return [
      {
        ...base,
        target: target("check", check.head_sha ?? check.sha ?? p.sha),
        feature: "checks",
        body: `${text(check.name ?? p.context ?? "Checks")} · ${text(check.conclusion ?? check.status ?? check.state)}`,
        mentions: [],
      },
    ];
  }
  return [];
}
export function summary(target: Target, s: Json): string {
  const state = s.merged
    ? "merged"
    : s.draft
      ? "draft"
      : (s.state ??
        s.conclusion ??
        s.status ??
        s.latest?.state ??
        (s.isAnswered ? "answered" : "open"));
  const title =
    s.title ??
    s.name ??
    s.tag_name ??
    s.commit?.message?.split("\n")[0] ??
    s.environment ??
    target.number;
  const labels = (Array.isArray(s.labels) ? s.labels : (s.labels?.nodes ?? []))
    .map((l: Json) => text(l.name, 80))
    .join(", ");
  const assignees = (s.assignees ?? []).map((a: Json) => text(a.login, 80)).join(", ");
  const reviewers = (s.requested_reviewers ?? []).map((a: Json) => text(a.login, 80)).join(", ");
  return Array.from(
    [
      `${text(target.repo, 160)} · ${target.kind} ${["issue", "pull", "discussion"].includes(target.kind) ? "#" : ""}${text(target.number, 100)} · ${text(state, 100)}`,
      text(title, 500),
      resource(target),
      text(s.body ?? s.latest?.description ?? s.description, 900),
      labels ? `Labels: ${labels}` : "",
      assignees ? `Assignees: ${assignees}` : "",
      reviewers ? `Review requested: ${reviewers}` : "",
      s.reviews ? `Reviews: ${text(s.reviews.join(" / "), 350)}` : "",
      typeof s.comments === "number"
        ? `Comments: ${s.comments}${typeof s.review_comments === "number" ? ` / Review comments: ${s.review_comments}` : ""}`
        : "",
      s.checkSummary ? `Checks: ${text(s.checkSummary, 500)}` : "",
    ]
      .filter(Boolean)
      .join("\n"),
  )
    .slice(0, 3800)
    .join("");
}
