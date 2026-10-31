import { appendFile, readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const colors = {
  success: "#2ea043",
  failure: "#cf222e",
  error: "#cf222e",
  pending: "#bf8700",
  queued: "#bf8700",
  in_progress: "#bf8700",
  inactive: "#0969da",
  unknown: "#6e7781",
};
const prefixes = {
  "Zli-UoA/AuthZ": "authz",
  "Zli-UoA/zoo2": "zoo2",
  "Zli-UoA/zoo2-infra": "infra",
};
const shortSha = (value) =>
  /^[a-f0-9]{40,64}$/.test(value ?? "") ? value.slice(0, 7) : "unknown";
const knownState = (value) =>
  Object.hasOwn(colors, value) ? value : "unknown";

export function validateConfig(config, repository) {
  if (
    !["app", "infra"].includes(config.mode) ||
    !Array.isArray(config.required) ||
    !config.required.length ||
    config.required.some((value) => typeof value !== "string" || !value)
  )
    throw new Error("Invalid badge status configuration");
  if (
    !prefixes[repository] ||
    config.prefix !== prefixes[repository] ||
    (config.mode === "infra") !== (config.prefix === "infra")
  )
    throw new Error("Badge prefix does not match repository");
  if (!/^[a-f0-9]{20,64}$/.test(config.gistId ?? ""))
    throw new Error("A valid badge Gist ID is required");
}

export function summarize(statuses, required, mode) {
  const latest = new Map();
  for (const status of statuses) {
    const key =
      mode === "infra"
        ? /^kustomization\/([^/]+)\//.exec(status.context)?.[1]
        : status.context;
    if (key && !latest.has(key)) latest.set(key, status);
  }
  const entries = required.map((name) => ({ name, ...latest.get(name) }));
  const state = entries.some((entry) =>
    ["failure", "error"].includes(entry.state),
  )
    ? "failure"
    : entries.every((entry) => entry.state === "success")
      ? "success"
      : "pending";
  return { state, entries };
}

export function makeApi(repository, token, request = fetch) {
  return async (path) => {
    const response = await request(
      `https://api.github.com/repos/${repository}/${path}`,
      {
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        signal: AbortSignal.timeout(20000),
        redirect: "error",
      },
    );
    if (!response.ok)
      throw new Error(
        `Repository status request failed: HTTP ${response.status}`,
      );
    return response.json();
  };
}

async function allPages(api, path) {
  const items = [];
  let firstPage;
  for (let page = 1; page <= 100; page++) {
    const chunk = await api(`${path}?per_page=100&page=${page}`);
    if (page === 1) firstPage = chunk;
    items.push(...chunk);
    if (chunk.length < 100) return { items, firstPage };
  }
  throw new Error("Status history exceeds pagination limit");
}

export async function collect(api, config) {
  // Status records are append-only and newest first. Rechecking the first page
  // detects inserts that could shift pagination boundaries during collection.
  const fingerprint = (statuses) =>
    JSON.stringify(
      statuses.map(({ id, context, state, updated_at }) => ({
        id,
        context,
        state,
        updated_at,
      })),
    );
  for (let attempt = 0; attempt < 3; attempt++) {
    const head = await api("commits/main");
    const { items, firstPage } = await allPages(
      api,
      `commits/${head.sha}/statuses`,
    );
    const result = {
      sha: head.sha,
      ...summarize(items, config.required, config.mode),
    };
    if (config.mode === "app") {
      const deployments = await api(
        "deployments?environment=production&per_page=1",
      );
      if (deployments.length) {
        const states = await api(
          `deployments/${deployments[0].id}/statuses?per_page=1`,
        );
        result.production = {
          sha: deployments[0].sha,
          state: states[0]?.state ?? "unknown",
          updated_at: states[0]?.created_at,
        };
      }
    }
    const latestStatuses = await api(
      `commits/${head.sha}/statuses?per_page=100&page=1`,
    );
    const latestHead = await api("commits/main");
    if (
      head.sha === latestHead.sha &&
      fingerprint(firstPage) === fingerprint(latestStatuses)
    )
      return result;
  }
  throw new Error(
    "Deployment status changed during all three collection attempts",
  );
}

export function endpoint(label, message, state) {
  return { schemaVersion: 1, label, message, color: colors[knownState(state)] };
}

export function filesFor(result, config, now = new Date().toISOString()) {
  const state = knownState(result.state);
  const badges = {
    main: endpoint("Flux main", `${state} ${shortSha(result.sha)}`, state),
    updated: endpoint(
      "checked UTC",
      now.replace("T", " ").slice(0, 16),
      "unknown",
    ),
  };
  if (config.mode === "app") {
    const production = result.production;
    const productionState = knownState(production?.state);
    badges.production = endpoint(
      "production",
      production ? `${productionState} ${shortSha(production.sha)}` : "unknown",
      productionState,
    );
  }
  // Send only this repository's prefix. Omitted Gist files stay unchanged;
  // never send other prefixes or null deletions. Select public fields only.
  return Object.fromEntries(
    Object.entries(badges).map(([kind, badge]) => [
      `${config.prefix}-${kind}.json`,
      { content: JSON.stringify(badge) + "\n" },
    ]),
  );
}

export function makeGistPublisher(
  gistId,
  token,
  {
    request = fetch,
    wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  } = {},
) {
  if (!/^[a-f0-9]{20,64}$/.test(gistId) || !token)
    throw new Error("Gist ID and BADGE_GIST_TOKEN are required");
  return async (files) => {
    for (let attempt = 0; attempt < 3; attempt++) {
      let response;
      try {
        response = await request(`https://api.github.com/gists/${gistId}`, {
          method: "PATCH",
          headers: {
            Authorization: `Bearer ${token}`,
            Accept: "application/vnd.github+json",
            "Content-Type": "application/json",
            "X-GitHub-Api-Version": "2022-11-28",
          },
          body: JSON.stringify({ files }),
          signal: AbortSignal.timeout(20000),
          redirect: "error",
        });
      } catch {
        if (attempt === 2)
          throw new Error("Gist update failed after three network attempts");
        await wait(1000 * 2 ** attempt);
        continue;
      }
      if (response.ok) return;
      if (attempt === 2 || (response.status !== 429 && response.status < 500))
        throw new Error(`Gist update failed: HTTP ${response.status}`);
      const retryAfter = Number(response.headers.get("retry-after"));
      await wait(
        Math.min(
          30000,
          Math.max(
            1000 * 2 ** attempt,
            Number.isFinite(retryAfter) ? retryAfter * 1000 : 0,
          ),
        ),
      );
    }
  };
}

const cell = (value) => String(value ?? "-").replace(/[\r\n|]/g, " ");
export function jobSummary(result, now) {
  const rows = (result.entries ?? [])
    .map(
      (entry) =>
        `| ${cell(entry.name)} | ${cell(entry.state ?? "pending")} | ${cell(entry.updated_at)} |`,
    )
    .join("\n");
  return `## Flux deployment snapshot\n\nChecked at ${now} (UTC).\n\nMain: ${cell(result.sha ?? "unknown")} — ${cell(result.state)}\n\n| Resource | State | Commit status updated (UTC) |\n| --- | --- | --- |\n${rows}\n\nProduction: ${cell(result.production?.sha ?? "unknown")} — ${cell(result.production?.state ?? "unknown")}\n`;
}

export async function run(
  api,
  publish,
  config,
  { now, summary = async () => {} } = {},
) {
  let result;
  try {
    result = await collect(api, config);
  } catch {
    result = { state: "unknown", entries: [] };
    console.error(
      "::warning::Could not read stable deployment status; publishing unknown",
    );
  }
  const checkedAt = now ?? new Date().toISOString();
  await publish(filesFor(result, config, checkedAt));
  await summary(jobSummary(result, checkedAt));
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const repository = process.env.GITHUB_REPOSITORY;
  const repoToken = process.env.GH_TOKEN;
  const gistToken = process.env.BADGE_GIST_TOKEN;
  if (!repository || !repoToken || !gistToken)
    throw new Error(
      "GITHUB_REPOSITORY, GH_TOKEN and BADGE_GIST_TOKEN are required",
    );
  const config = JSON.parse(
    await readFile(".github/deployment-badges.json", "utf8"),
  );
  validateConfig(config, repository);
  await run(
    makeApi(repository, repoToken),
    makeGistPublisher(config.gistId, gistToken),
    config,
    {
      summary: process.env.GITHUB_STEP_SUMMARY
        ? (text) => appendFile(process.env.GITHUB_STEP_SUMMARY, text)
        : undefined,
    },
  );
}
