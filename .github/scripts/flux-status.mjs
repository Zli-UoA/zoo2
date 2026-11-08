import { readFile, rename, writeFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const repository = "Zli-UoA/zoo2";
const packagePath = "/orgs/Zli-UoA/packages/container/zoo2-migrate/versions";
const environment = "production";

export function parseEvent(event) {
  const payload = event.client_payload;
  const object = payload?.involvedObject;
  if (
    object?.kind !== "Kustomization" ||
    object.namespace !== "flux-system" ||
    !["zoo2", "zoo2-migrate"].includes(object.name) ||
    event.action !== `Kustomization/${object.name}.flux-system` ||
    payload.reportingController !== "kustomize-controller"
  ) {
    throw new Error("Unexpected Flux event source");
  }
  // Dispatch forwards intermediate info events too. Only reconciliation
  // completion means that the workload health checks passed. Progressing
  // events are reported on the Check Run only; they never touch the commit
  // status, the Deployment or the ordering watermark.
  const progressing =
    payload.reason === "Progressing" ||
    payload.reason === "ProgressingWithRetry";
  if (
    !progressing &&
    payload.severity !== "error" &&
    !(
      payload.severity === "info" &&
      payload.reason === "ReconciliationSucceeded"
    )
  )
    return null;
  const releaseTag = payload.metadata?.["release-tag"];
  const digest = payload.metadata?.digest;
  const validTimestamp =
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/.test(
      payload.timestamp ?? "",
    ) && Number.isFinite(Date.parse(payload.timestamp));
  // A progress event whose release is not identifiable yet is just noise.
  if (
    progressing &&
    (!/^0\.0\.[1-9][0-9]{0,14}$/.test(releaseTag ?? "") ||
      !/^sha256:[a-f0-9]{64}$/.test(digest ?? "") ||
      !validTimestamp)
  )
    return null;
  if (!/^0\.0\.[1-9][0-9]{0,14}$/.test(releaseTag ?? ""))
    throw new Error("Invalid release tag");
  if (!/^sha256:[a-f0-9]{64}$/.test(digest ?? ""))
    throw new Error("Invalid image digest");
  if (!validTimestamp) throw new Error("Invalid event timestamp");
  return {
    name: object.name,
    releaseTag,
    digest,
    timestamp: payload.timestamp,
    state: progressing
      ? "pending"
      : payload.severity === "error"
        ? "failure"
        : "success",
    context: `flux/${object.name}`,
    reason: payload.reason,
    message: payload.message,
  };
}

export function createApi(token, fetcher = fetch) {
  return async (path, body, method = body === undefined ? "GET" : "POST") => {
    const response = await fetcher(`https://api.github.com${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.timeout(30_000),
    });
    if (!response.ok)
      throw new Error(`GitHub API ${response.status}: ${path.split("?")[0]}`);
    return response.status === 204 ? null : response.json();
  };
}

async function* pages(api, path) {
  for (let page = 1; ; page++) {
    const values = await api(
      `${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`,
    );
    if (!Array.isArray(values))
      throw new Error("Expected a paginated GitHub API response");
    yield* values;
    if (values.length < 100) return;
  }
}

export async function resolveSha(api, digest, releaseTag) {
  for await (const version of pages(api, packagePath)) {
    if (version.name !== digest) continue;
    const tags = version.metadata?.container?.tags ?? [];
    if (!tags.includes(releaseTag)) return null;
    const shas = [...new Set(tags.filter((tag) => /^[a-f0-9]{40}$/.test(tag)))];
    if (shas.length > 1)
      throw new Error("Image digest has multiple commit SHA tags");
    return shas[0] ?? null;
  }
  return null;
}

export function parseState(text) {
  const state = JSON.parse(text);
  if (
    state?.version !== 1 ||
    !state.workloads ||
    typeof state.workloads !== "object" ||
    Array.isArray(state.workloads)
  )
    throw new Error("Invalid Flux watermark");
  for (const [name, value] of Object.entries(state.workloads)) {
    if (
      !["zoo2", "zoo2-migrate"].includes(name) ||
      !Number.isFinite(Date.parse(value?.timestamp)) ||
      // Snapshots saved before outcomes were recorded have no state.
      ![undefined, "success", "failure"].includes(value.state) ||
      // The previous release is recorded to tell a rollback from a rerun.
      (value.digest !== undefined &&
        !/^sha256:[a-f0-9]{64}$/.test(value.digest)) ||
      (value.releaseTag !== undefined &&
        !/^0\.0\.[1-9][0-9]{0,14}$/.test(value.releaseTag))
    )
      throw new Error("Invalid Flux watermark");
  }
  return state;
}

// Only restore artifacts produced by this trusted default-branch workflow.
// A PR job with the same artifact name must not become a source of state.
async function stateArtifacts(api) {
  const artifacts = [];
  for (let page = 1; ; page++) {
    const data = await api(
      `/repos/${repository}/actions/artifacts?name=flux-production-state&per_page=100&page=${page}`,
    );
    artifacts.push(...data.artifacts.filter((artifact) => !artifact.expired));
    if (data.artifacts.length < 100) break;
  }
  // A retry of an older run may have the freshest snapshot. Do not depend on
  // workflow run numbers or the API's implicit ordering of artifact pages.
  artifacts.sort(
    (a, b) =>
      (Date.parse(b.created_at) || 0) - (Date.parse(a.created_at) || 0) ||
      (b.id || 0) - (a.id || 0),
  );
  return artifacts;
}

function trustedStateRun(run) {
  return (
    run.event === "repository_dispatch" &&
    run.head_branch === "main" &&
    [
      ".github/workflows/flux-status.yaml",
      ".github/workflows/flux-status.yaml@main",
    ].includes(run.path) &&
    run.head_repository?.full_name?.toLowerCase() === repository.toLowerCase()
  );
}

export async function findStateRun(api) {
  for (const artifact of await stateArtifacts(api)) {
    const id = artifact.workflow_run?.id;
    if (!Number.isSafeInteger(id))
      throw new Error("Invalid artifact workflow run ID");
    const run = await api(`/repos/${repository}/actions/runs/${id}`);
    if (trustedStateRun(run)) return id;
  }
  return null;
}

// Collect before deleting so pagination cannot shift and skip records.
// Keep two trusted snapshots and never delete another workflow's data.
export async function pruneStateArtifacts(api) {
  let kept = 0;
  for (const artifact of await stateArtifacts(api)) {
    const id = artifact.workflow_run?.id;
    if (!Number.isSafeInteger(id) || !Number.isSafeInteger(artifact.id))
      throw new Error("Invalid artifact ID");
    if (!trustedStateRun(await api(`/repos/${repository}/actions/runs/${id}`)))
      continue;
    if (++kept <= 2) continue;
    await api(
      `/repos/${repository}/actions/artifacts/${artifact.id}`,
      undefined,
      "DELETE",
    );
  }
}

const maxCheckText = 65535;
const omittedMarker = "(older events omitted)";
const fence = "```";
const checkTitles = {
  pending: (tag) => `Rolling out ${tag}`,
  success: (tag) => `Rolled out ${tag}`,
  failure: (tag) => `Rollout of ${tag} failed`,
};

// Both fields end up in a Markdown code block and in a size-capped text.
// Reason is not trusted to be a fixed word: error events can carry free text.
function clean(value, max) {
  return (
    String(value ?? "")
      // eslint-disable-next-line no-control-regex
      .replace(/[\u0000-\u001f\u007f]+/g, " ")
      // Keep a value from closing the code block.
      .replace(/`/g, "'")
      .trim()
      .slice(0, max)
  );
}

// zoo2 is a public repository: Check Runs and workflow logs are readable by
// anyone. Flux messages can name cluster objects, so they are not published
// until real production events have been checked. Reasons are normally a
// fixed vocabulary, but error events can carry free text, so they are cleaned
// and capped anyway. Set this to true only after that check.
export const publishMessages = false;

function eventLine(result, messages = publishMessages) {
  const reason = clean(result.reason, 100);
  const message = messages ? clean(result.message, 200) : "";
  return `${result.timestamp} ${reason}${message ? `: ${message}` : ""}`;
}

// output.text is Markdown. A code block keeps one event per line and stops
// "_" or "*" in Flux messages from being rendered as emphasis.
function readCheckLines(text) {
  const lines = String(text ?? "")
    .replace(/^```\n?/, "")
    .replace(/\n?```$/, "")
    .split("\n")
    .filter(Boolean);
  const omitted = lines[0] === omittedMarker;
  return { lines: omitted ? lines.slice(1) : lines, omitted };
}

// output.text is replaced, not appended, and is capped. Drop the oldest lines.
export function fitCheckText(lines, omitted = false) {
  const wrap = (body) => `${fence}\n${body}\n${fence}`;
  const room = maxCheckText - wrap("").length;
  const all = omitted ? [omittedMarker, ...lines] : lines;
  if (all.join("\n").length <= room) return wrap(all.join("\n"));
  let budget = room - omittedMarker.length - 1;
  const kept = [];
  for (let i = lines.length - 1; i >= 0; i--) {
    budget -= lines[i].length + 1;
    if (budget < 0 && kept.length > 0) break;
    kept.unshift(lines[i]);
  }
  // A single line longer than the cap cannot be kept whole.
  return wrap([omittedMarker, ...kept].join("\n").slice(0, room));
}

const byStart = (a, b) =>
  (Date.parse(a.started_at) || 0) - (Date.parse(b.started_at) || 0) ||
  (a.id || 0) - (b.id || 0);

// All Check Runs of this rollout, oldest first. A rollback to an earlier
// release adds runs with the same base id and a "@<time>" suffix.
async function findCheckRuns(api, sha, name, base) {
  const found = [];
  for (let page = 1; ; page++) {
    const data = await api(
      `/repos/${repository}/commits/${sha}/check-runs?check_name=${encodeURIComponent(name)}&filter=all&per_page=100&page=${page}`,
    );
    const runs = data?.check_runs;
    if (!Array.isArray(runs))
      throw new Error("Expected a check_runs GitHub API response");
    for (const run of runs) {
      const id = run.external_id;
      if (id === base || id?.startsWith(`${base}@`)) found.push(run);
    }
    if (runs.length < 100) return found.sort(byStart);
  }
}

// The run an event belongs to: the latest one that started no later than the
// event. A delayed event never updates a run that began after it.
const runFor = (runs, timestamp) =>
  runs
    .filter((run) => !(Date.parse(run.started_at) > Date.parse(timestamp)))
    .at(-1) ?? null;

// One Check Run per rollout (name + release tag + digest). Events add a line
// to its text. A completed run is never reopened by a delayed progress event,
// and a repeated conclusion (interval reconciliation) changes nothing. When
// the previous release recorded in the watermark differs, an event that
// belongs to an older run starts a rollback (A -> B -> A) with its own Check
// Run.
export async function recordCheckRun(
  api,
  { sha, result, runUrl, previous, messages },
) {
  const done = result.state !== "pending";
  const base = `${result.name}:${result.releaseTag}:${result.digest}`;
  const runs = await findCheckRuns(api, sha, result.context, base);
  let existing = runFor(runs, result.timestamp);
  // Every run started after this event: the event is older than all of them.
  if (!existing && runs.length > 0) return { checkRun: "unchanged" };
  // A rerun after a partial failure (Deployment API error, lost artifact
  // save) sees the old watermark again. The run of the very same rollout is
  // newer than that watermark, so it is reused. Only a run that ended (or, for
  // progress, started) before the last recorded completion belongs to an
  // earlier rollout of this release, which is now being rolled back to. A
  // completion never starts a new run for an unfinished one: it ends it.
  const differs =
    previous?.digest !== undefined &&
    (previous.digest !== result.digest ||
      previous.releaseTag !== result.releaseTag);
  const before =
    Date.parse(existing?.completed_at ?? existing?.started_at) <
    Date.parse(previous?.timestamp);
  let newRun =
    differs &&
    before &&
    (existing?.status === "completed" ||
      (!done && existing?.status === "in_progress"));
  if (newRun && !done) {
    // Progress events of one rollback can arrive out of order. If a run that
    // started after the last recorded completion already exists, this event
    // belongs to that rollback: add to it (never reopening a finished one)
    // instead of starting a second run.
    const rollback = runs.find(
      (run) => !(Date.parse(run.started_at) < Date.parse(previous.timestamp)),
    );
    if (rollback) {
      existing = rollback;
      newRun = false;
    }
  }
  if (newRun) existing = null;
  if (
    existing?.status === "completed" &&
    (!done || existing.conclusion === result.state)
  )
    return { checkRun: "unchanged" };
  const line = eventLine(result, messages);
  const current = readCheckLines(existing?.output?.text);
  const has = current.lines.includes(line);
  if (existing && has && !done) return { checkRun: "unchanged" };
  if (!has) current.lines.push(line);
  const output = {
    title: checkTitles[result.state](result.releaseTag),
    summary: `${result.name} ${result.releaseTag}\n${result.digest}\n[Workflow run](${runUrl})`,
    text: fitCheckText(current.lines, current.omitted),
  };
  const finish = done
    ? { conclusion: result.state, completed_at: result.timestamp }
    : {};
  const status = done ? "completed" : "in_progress";
  if (existing) {
    await api(
      `/repos/${repository}/check-runs/${existing.id}`,
      { status, ...finish, output },
      "PATCH",
    );
    return { checkRun: "updated" };
  }
  await api(`/repos/${repository}/check-runs`, {
    name: result.context,
    head_sha: sha,
    external_id: newRun ? `${base}@${result.timestamp}` : base,
    status,
    started_at: result.timestamp,
    details_url: runUrl,
    ...finish,
    output,
  });
  return { checkRun: "created" };
}

// A completion event dropped as older than the watermark must still end the
// Check Run its own rollout left in_progress. It never opens, creates or
// reopens one, and it leaves a run that started after the event alone.
export async function closeStaleCheckRun(
  api,
  { sha, result, runUrl, messages },
) {
  sha ??= await verifiedSha(api, result);
  if (!sha) return { checkRun: "unchanged" };
  const base = `${result.name}:${result.releaseTag}:${result.digest}`;
  const runs = await findCheckRuns(api, sha, result.context, base);
  if (runFor(runs, result.timestamp)?.status !== "in_progress")
    return { checkRun: "unchanged" };
  return recordCheckRun(api, { sha, result, runUrl, messages });
}

async function verifiedSha(api, result) {
  const sha = await resolveSha(api, result.digest, result.releaseTag);
  if (!sha) return null;
  // A SHA tag is not necessarily a commit in this repository.
  const commit = await api(`/repos/${repository}/commits/${sha}`);
  if (commit.sha !== sha)
    throw new Error("Image SHA does not match an zoo2 commit");
  return sha;
}

// The log of a public repository must not carry Flux messages either: the
// result of report() includes the event message, so it is dropped here.
export function logLine(result) {
  return JSON.stringify(result, (key, value) =>
    key === "message" ? undefined : value,
  );
}

export async function report(
  event,
  {
    api,
    runUrl,
    messages,
    warn = console.warn,
    state = { version: 1, workloads: {} },
  },
) {
  const result = parseEvent(event);
  if (!result) return { skipped: "intermediate" };
  const previous = state.workloads[result.name];
  const previousTime = Date.parse(previous?.timestamp);
  const resultTime = Date.parse(result.timestamp);
  if (previousTime > resultTime) {
    if (result.state === "pending") return { skipped: "older-event" };
    return {
      skipped: "older-event",
      ...(await closeStaleCheckRun(api, { result, runUrl, messages })),
    };
  }
  if (result.state === "pending") {
    const sha = await verifiedSha(api, result);
    if (!sha) {
      warn(
        "::warning::No matching release tag, digest and commit SHA found; nothing was reported.",
      );
      return { skipped: "missing-sha" };
    }
    return {
      sha,
      ...result,
      ...(await recordCheckRun(api, {
        sha,
        result,
        runUrl,
        previous,
        messages,
      })),
    };
  }
  // Flux event timestamps have second resolution and dispatch order is not
  // event order. A success and a failure in the same second cannot be ordered,
  // so never let such a success replace the failure (or create a Deployment).
  // The next reconciliation reports a later success if the workload recovered.
  if (
    previousTime === resultTime &&
    previous.state === "failure" &&
    result.state === "success"
  ) {
    warn(
      "::warning::A success with the same timestamp as a recorded failure cannot be ordered; nothing was reported.",
    );
    return { skipped: "ambiguous-same-time" };
  }
  const remember = () => {
    state.workloads[result.name] = {
      timestamp: result.timestamp,
      state: result.state,
      releaseTag: result.releaseTag,
      digest: result.digest,
    };
  };
  const sha = await resolveSha(api, result.digest, result.releaseTag);
  if (!sha) {
    remember();
    warn(
      "::warning::No matching release tag, digest and commit SHA found; nothing was reported.",
    );
    return { skipped: "missing-sha" };
  }
  // A SHA tag is not necessarily a commit in this repository.
  const commit = await api(`/repos/${repository}/commits/${sha}`);
  if (commit.sha !== sha)
    throw new Error("Image SHA does not match an zoo2 commit");
  // Stable descriptions avoid consuming the 1,000 statuses per SHA/context
  // limit on interval reconciliations or repeated failures. The artifact stores
  // the latest observed time independently, including no-op reconciliations.
  const description = `${result.name}: ${result.state} ${result.releaseTag} ${result.digest}`;
  let latest;
  for await (const status of pages(
    api,
    `/repos/${repository}/commits/${sha}/statuses`,
  )) {
    if (status.context === result.context) {
      latest = status;
      break;
    }
  }
  const latestTime = Date.parse(latest?.description?.split(" ")[0]); // legacy format
  if (latestTime > Date.parse(result.timestamp))
    return {
      skipped: "older-event",
      ...(await closeStaleCheckRun(api, { sha, result, runUrl, messages })),
    };
  if (latest?.description !== description || latest?.state !== result.state) {
    await api(`/repos/${repository}/statuses/${sha}`, {
      state: result.state,
      context: result.context,
      description,
      target_url: runUrl,
    });
  }
  const check = await recordCheckRun(api, {
    sha,
    result,
    runUrl,
    previous,
    messages,
  });
  if (result.name !== "zoo2" || result.state !== "success") {
    remember();
    return { sha, ...result, ...check };
  }

  // Resume a partially completed run without creating a second Deployment.
  let deployment;
  for await (const existing of pages(
    api,
    `/repos/${repository}/deployments?environment=${environment}`,
  )) {
    if (existing.payload?.source !== "flux/zoo2") continue;
    if (Date.parse(existing.payload.timestamp) > Date.parse(result.timestamp)) {
      remember();
      return { sha, ...result, ...check, skipped: "older-deployment" };
    }
    // Compare only the latest Flux production rollout. A -> B -> A is a real
    // rollback and deserves a new record; A -> A is just a reconciliation.
    if (
      existing.sha === sha &&
      existing.payload.digest === result.digest &&
      existing.payload.releaseTag === result.releaseTag
    )
      deployment = existing;
    break;
  }
  deployment ??= await api(`/repos/${repository}/deployments`, {
    ref: sha,
    environment,
    auto_merge: false,
    required_contexts: [],
    production_environment: true,
    transient_environment: false,
    description: `Flux zoo2 ${result.timestamp}`,
    payload: {
      source: "flux/zoo2",
      releaseTag: result.releaseTag,
      digest: result.digest,
      timestamp: result.timestamp,
    },
  });
  if (!deployment.id) throw new Error("GitHub did not return a Deployment ID");
  const statuses = await api(
    `/repos/${repository}/deployments/${deployment.id}/statuses?per_page=1`,
  );
  if (statuses[0]?.state !== "success") {
    await api(`/repos/${repository}/deployments/${deployment.id}/statuses`, {
      state: "success",
      environment,
      environment_url: "https://zli.chat",
      log_url: runUrl,
      description: `Flux zoo2 ${result.timestamp}`,
      auto_inactive: true,
    });
  }
  remember();
  return { sha, ...result, ...check };
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  try {
    if (!process.env.GH_TOKEN) throw new Error("GH_TOKEN is required");
    const api = createApi(process.env.GH_TOKEN);
    if (process.argv[2] === "--state-run") {
      console.log((await findStateRun(api)) ?? "");
    } else if (process.argv[2] === "--prune-state") {
      await pruneStateArtifacts(api);
    } else {
      let state = { version: 1, workloads: {} };
      if (!process.env.FLUX_STATE_PATH)
        throw new Error("FLUX_STATE_PATH is required");
      try {
        state = parseState(await readFile(process.env.FLUX_STATE_PATH, "utf8"));
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
      }
      const event = JSON.parse(
        await readFile(process.env.GITHUB_EVENT_PATH, "utf8"),
      );
      const result = await report(event, {
        api,
        state,
        runUrl: `https://github.com/${repository}/actions/runs/${process.env.GITHUB_RUN_ID}`,
      });
      await writeFile(
        process.env.FLUX_STATE_PATH + ".tmp",
        JSON.stringify(state) + "\n",
      );
      await rename(
        process.env.FLUX_STATE_PATH + ".tmp",
        process.env.FLUX_STATE_PATH,
      );
      console.log(logLine(result));
    }
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exitCode = 1;
  }
}
