import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const colors = {
  success: "#2ea043",
  failure: "#cf222e",
  pending: "#bf8700",
  unknown: "#6e7781",
};
export const escapeXml = (value) =>
  String(value).replace(
    /[&<>"']/g,
    (c) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&apos;",
      })[c],
  );

export function svg(label, message, state) {
  const left = 20 + label.length * 7;
  const right = 20 + message.length * 7;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${left + right}" height="20" role="img" aria-label="${escapeXml(label + ": " + message)}"><title>${escapeXml(label + ": " + message)}</title><rect width="${left}" height="20" fill="#24292f"/><rect x="${left}" width="${right}" height="20" fill="${colors[state] ?? colors.unknown}"/><g fill="white" font-family="Verdana,DejaVu Sans,sans-serif" font-size="11" text-anchor="middle"><text x="${left / 2}" y="14">${escapeXml(label)}</text><text x="${left + right / 2}" y="14">${escapeXml(message)}</text></g></svg>\n`;
}

export function summarize(statuses, required, mode) {
  const latest = new Map();
  // The statuses endpoint returns newest entries first, including repeated contexts.
  for (const status of statuses) {
    const key =
      mode === "infra"
        ? /^kustomization\/([^/]+)\//.exec(status.context)?.[1]
        : status.context;
    if (key && !latest.has(key)) latest.set(key, status);
  }
  const entries = required.map((name) => ({ name, ...latest.get(name) }));
  const state = entries.some((s) => ["failure", "error"].includes(s.state))
    ? "failure"
    : entries.every((s) => s.state === "success") && entries.length > 0
      ? "success"
      : "pending";
  return { state, entries };
}

export function makeApi(repository, token, request = fetch) {
  return async (path, method = "GET", body) => {
    const response = await request(
      `https://api.github.com/repos/${repository}/${path}`,
      {
        method,
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: "application/vnd.github+json",
          "Content-Type": "application/json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      },
    );
    if (!response.ok) {
      const error = new Error(
        `GitHub ${method} ${path.split("?")[0]}: HTTP ${response.status}`,
      );
      error.status = response.status;
      throw error;
    }
    return response.json();
  };
}

async function allPages(api, path) {
  const items = [];
  for (let page = 1; ; page++) {
    const chunk = await api(
      `${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`,
    );
    items.push(...chunk);
    if (chunk.length < 100) return items;
    if (page >= 100) throw new Error("Status history exceeds pagination limit");
  }
}

export async function collect(api, config) {
  const head = await api("commits/main");
  const statuses = await allPages(api, `commits/${head.sha}/statuses`);
  const summary = summarize(statuses, config.required, config.mode);
  const result = { sha: head.sha, ...summary };
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
  return result;
}

export function filesFor(result, repository, now = new Date().toISOString()) {
  const short = result.sha?.slice(0, 7) ?? "unknown";
  const state = result.state;
  const files = {
    "main.svg": svg("Flux main", `${state} ${short}`, state),
    "updated.svg": svg(
      "checked UTC",
      now.replace("T", " ").slice(0, 16),
      "unknown",
    ),
    "status.json":
      JSON.stringify({ ...result, checked_at: now }, null, 2) + "\n",
  };
  if (result.production) {
    const prod = result.production;
    files["production.svg"] = svg(
      "production",
      `${prod.state} ${prod.sha.slice(0, 7)}`,
      prod.state,
    );
  } else {
    files["production.svg"] = svg("production", "unknown", "unknown");
  }
  const link = `https://github.com/${repository}`;
  const rows = (result.entries ?? [])
    .map(
      (entry) =>
        `| ${entry.name} | ${entry.state ?? "pending"} | ${entry.updated_at ?? "-"} |`,
    )
    .join("\n");
  files["README.md"] =
    `# Deployment status\n\nChecked at ${now} (UTC). This is a snapshot of GitHub's Flux notifications, not a live availability check.\n\nMain: ${result.sha ? `[${short}](${link}/commit/${result.sha})` : "unknown"} — ${state}\n\n| Flux resource | State | Last event (UTC) |\n| --- | --- | --- |\n${rows}\n\n${result.production ? `Production: [${result.production.sha.slice(0, 7)}](${link}/commit/${result.production.sha}), ${result.production.state}, last event ${result.production.updated_at ?? "unknown"}.` : "Production deployment: unknown."}\n\n[Deployment history](${link}/deployments/production) · [Badge updater](${link}/actions/workflows/deployment-badges.yaml)\n`;
  return files;
}

export async function publish(api, files) {
  const branch = "deployment-badges";
  let previous;
  try {
    previous = await api(`git/ref/heads/${branch}`);
  } catch (error) {
    if (error.status !== 404) throw error;
  }
  const tree = await api("git/trees", "POST", {
    tree: Object.entries(files).map(([path, content]) => ({
      path,
      mode: "100644",
      type: "blob",
      content,
    })),
  });
  const commit = await api("git/commits", "POST", {
    message: "chore: update deployment badges [skip ci]",
    tree: tree.sha,
    parents: previous ? [previous.object.sha] : [],
  });
  if (previous)
    await api(`git/refs/heads/${branch}`, "PATCH", {
      sha: commit.sha,
      force: false,
    });
  else
    await api("git/refs", "POST", {
      ref: `refs/heads/${branch}`,
      sha: commit.sha,
    });
}

export async function run(api, config, repository) {
  let result;
  try {
    result = await collect(api, config);
  } catch {
    // Never retain a successful badge when collection fails. Do not log API bodies.
    result = { state: "unknown", entries: [] };
    console.error(
      "::warning::Could not read deployment status; publishing unknown",
    );
  }
  await publish(api, filesFor(result, repository));
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const repository = process.env.GITHUB_REPOSITORY;
  const token = process.env.GH_TOKEN;
  if (!repository || !token)
    throw new Error("GITHUB_REPOSITORY and GH_TOKEN are required");
  const config = JSON.parse(
    await readFile(".github/deployment-badges.json", "utf8"),
  );
  await run(makeApi(repository, token), config, repository);
}
