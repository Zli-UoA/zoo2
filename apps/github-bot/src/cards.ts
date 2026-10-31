import { text, resource, type Json } from "./types.ts";
import type { Notice } from "./events.ts";

/** Case-insensitive keys stay internal; GitHub's canonical spelling is presentation data. */
export function displayRepository(repo: string, payload: Json): string {
  const original = payload.repository?.full_name;
  return typeof original === "string" && original.toLowerCase() === repo.toLowerCase()
    ? original
    : repo;
}
function https(value: unknown): string {
  try {
    const u = new URL(String(value));
    if (u.protocol === "https:" && !u.username && !u.password) return u.toString();
  } catch {
    /* Omit unavailable or unsafe links. */
  }
  return "";
}
export function messageBlocks(notice: Notice, s: Json, payload: Json, reply = false): Json[] {
  // The parent is canonical current state; progress replies record the delivered transition.
  if (reply && notice.target.kind === "workflow" && payload.workflow_run)
    s = { ...s, ...payload.workflow_run };
  if (reply && notice.target.kind === "deployment" && payload.deployment_status)
    s = {
      ...s,
      ...payload.deployment,
      latest: payload.deployment_status,
      deploymentWorkflow:
        s.latest?.log_url === payload.deployment_status.log_url ? s.deploymentWorkflow : undefined,
    };
  const repo = displayRepository(notice.target.repo, payload);
  const target = { ...notice.target, repo };
  const state = String(
    s.merged
      ? "merged"
      : s.draft
        ? "draft"
        : (s.latest?.state ??
          s.conclusion ??
          s.status ??
          s.state ??
          (s.isAnswered ? "answered" : "open")),
  );
  const success = ["success", "completed", "active", "published", "open", "answered"].includes(
    state,
  );
  const failed = ["failure", "error", "cancelled", "timed_out", "closed", "deleted"].includes(
    state,
  );
  const color =
    state === "merged" ? "#8250df" : failed ? "#cf222e" : success ? "#2da44e" : "#bf8700";
  const actor = text(payload.sender?.login ?? "GitHub", 100);
  const fields: Json[] = [];
  const field = (title: string, value: unknown, short = false, url = "", code = false) => {
    if (value) fields.push({ title, value: text(value, 1000), short, url: https(url), code });
  };
  const kind = target.kind;
  let title = text(s.title ?? s.name ?? s.tag_name ?? s.environment ?? target.number, 450);
  let url = https(s.html_url ?? (kind === "discussion" ? s.url : "")) || resource(target);
  let description = text(s.body ?? s.description ?? "", 1500);
  let heading = `${kind === "pull" ? "Pull request" : kind === "issue" ? "Issue" : kind} ${String(notice.action ?? "updated").replaceAll("_", " ")} by ${actor}`;
  if (["issue", "pull", "discussion"].includes(kind)) {
    title = `#${target.number} ${title}`;
    field("Status", state, true);
    field("Assignees", (s.assignees ?? []).map((a: Json) => a.login).join(", "), true);
    field(
      "Review requested",
      (s.requested_reviewers ?? []).map((a: Json) => a.login).join(", "),
      true,
    );
    field("Reviews", s.reviews?.join(" / "), true);
    field("Checks", s.checkSummary);
    field(
      "Labels",
      (Array.isArray(s.labels) ? s.labels : (s.labels?.nodes ?? []))
        .map((l: Json) => l.name)
        .join(", "),
    );
    if (s.merged && notice.action === "closed") heading = `Pull request merged by ${actor}`;
  } else if (kind === "deployment") {
    const deployment = payload.deployment ?? s;
    title = `Deployment to ${text(deployment.environment ?? "environment", 200)}`;
    heading = `${title} by ${actor}`;
    const sha = String(deployment.sha ?? "");
    const ref = String(deployment.ref ?? "");
    field("Status", state === "success" ? "Completed" : state, true);
    field(
      "Commit",
      sha ? `${sha.slice(0, 7)}${ref && ref !== sha ? ` (${ref})` : ""}` : ref,
      true,
      sha ? `https://github.com/${repo}/commit/${encodeURIComponent(sha)}` : "",
      true,
    );
    const workflow = deployment.payload;
    if (s.deploymentWorkflow)
      field("Workflow", s.deploymentWorkflow.title, false, s.deploymentWorkflow.url);
    if (!s.deploymentWorkflow && workflow && typeof workflow === "object")
      field(
        "Workflow",
        workflow.workflow ?? workflow.name,
        false,
        workflow.workflow_url ?? workflow.html_url,
      );
    field("Details", s.latest?.description);
    url =
      https(s.latest?.environment_url ?? payload.deployment_status?.environment_url) ||
      resource(target);
    description = "";
  } else if (kind === "workflow") {
    title = `${text(s.name ?? "Workflow", 300)} #${s.run_number ?? target.number}`;
    field("Status", state, true);
    field(
      "Commit",
      String(s.head_sha ?? "").slice(0, 7),
      true,
      s.head_sha ? `https://github.com/${repo}/commit/${encodeURIComponent(s.head_sha)}` : "",
      true,
    );
    field("Branch", s.head_branch, true);
    field("Event", s.event, true);
    description = "";
  } else if (kind === "commit") {
    const count = payload.size ?? payload.commits?.length ?? 1;
    heading = `${count} new commit${count === 1 ? "" : "s"} pushed to ${text(String(payload.ref ?? "").replace(/^refs\/heads\//, ""))} by ${actor}`;
    title = `${repo} · ${target.number.slice(0, 7)}`;
    description = "";
    for (const commit of (payload.commits ?? []).slice(-8))
      field(
        String(commit.id).slice(0, 7),
        text(commit.message, 400),
        false,
        `https://github.com/${repo}/commit/${encodeURIComponent(commit.id)}`,
      );
  } else {
    field("Status", state, true);
    if (kind === "check") field("Checks", s.checkSummary);
  }
  if (reply) {
    if (["issue", "pull", "discussion"].includes(kind)) fields.length = 0;
    if (["comments", "reviews", "discussions"].includes(notice.feature)) {
      heading = notice.body.split("\n")[0];
      description = text(
        notice.body
          .split("\n")
          .slice(1)
          .filter((line) => line !== notice.url)
          .join("\n"),
        2000,
      );
      url = notice.url ?? url;
    } else description = "";
  }
  const timestamp =
    (notice.target.kind === "deployment" ? s.latest?.created_at : undefined) ??
    s.updated_at ??
    s.published_at ??
    payload.deployment_status?.created_at ??
    payload.repository?.updated_at;
  return [
    { type: "text", text: heading },
    {
      type: "card",
      card: {
        title: text(title, 500),
        description,
        url,
        color: kind === "commit" ? "#24292f" : color,
        source: text(repo, 160),
        context: text(timestamp, 40),
        badge: "",
        author: "",
        fields,
      },
    },
  ];
}
