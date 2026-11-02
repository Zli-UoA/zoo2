import { GitHub } from "./github.ts";
import { text, ruleFor, repoName, type Config, type Json, type Target } from "./types.ts";
export interface Preview {
  url: string;
  card: Json;
}
export async function preview(
  config: Config,
  github: GitHub,
  url: string,
  channel: string,
): Promise<Preview | undefined> {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return;
  }
  if (u.protocol !== "https:" || u.hostname !== "github.com" || u.port || u.username || u.password)
    return;
  const parts = u.pathname
    .split("/")
    .filter(Boolean)
    .map((s) => {
      try {
        return decodeURIComponent(s);
      } catch {
        return "";
      }
    });
  if (parts.some((s) => s === "." || s === ".." || /[\x00-\x1f]/.test(s))) return;
  if (parts.length < 2) {
    const owner = parts[0],
      rule = config.rules.find((r) => r.channels.includes(channel));
    if (!owner || !/^[a-z\d-]{1,39}$/i.test(owner) || !rule) return;
    const profile = await github.api(
      `/users/${encodeURIComponent(owner)}`,
      await github.installation(rule.repository),
    );
    return {
      url,
      card: {
        title: text(profile.name ?? profile.login, 500),
        description: text(profile.bio ?? profile.description, 1000),
        source: "GitHub",
        url: `https://github.com/${encodeURIComponent(owner)}`,
      },
    };
  }
  let repo: string;
  try {
    repo = repoName(`${parts[0]}/${parts[1]}`);
  } catch {
    return;
  }
  if (!ruleFor(config, repo, channel)) return;
  const encode = (v: string) => encodeURIComponent(v);
  // Admin-configured repository/channel permissions apply equally to notifications and previews.
  const metadata = await github.read(repo, "");
  const displayRepo = String(metadata.full_name ?? repo);
  u.pathname = u.pathname.replace(/^\/[^/]+\/[^/]+/, () => `/${displayRepo}`);
  let item: Json = metadata;
  let title: string = metadata.full_name;
  let description: string = metadata.description ?? "";
  let badge = "";
  let target: Target | undefined;
  if (["issues", "pull", "discussions"].includes(parts[2]) && /^[1-9]\d*$/.test(parts[3] ?? "")) {
    target = {
      repo,
      kind: parts[2] === "pull" ? "pull" : parts[2] === "issues" ? "issue" : "discussion",
      number: parts[3],
    };
    item = await github.snapshot(target);
    title = `${displayRepo} #${parts[3]} ${item.title}`;
    description = item.body ?? "";
    badge = item.merged ? "merged" : (item.state ?? (item.isAnswered ? "answered" : "open"));
    const comment = /^#issuecomment-(\d+)$/.exec(u.hash),
      review = /^#discussion_r(\d+)$/.exec(u.hash);
    if (comment || review) {
      const c = await github.read(
        repo,
        comment ? `/issues/comments/${comment[1]}` : `/pulls/comments/${review![1]}`,
      );
      const source = String(c.issue_url ?? c.pull_request_url ?? "");
      if (!source.endsWith(`/${parts[3]}`)) return;
      description = c.body ?? "";
    }
  } else if (parts[2] === "commit" && /^[a-f\d]{7,40}$/i.test(parts[3] ?? "")) {
    item = await github.read(repo, `/commits/${encode(parts[3])}`);
    title = `${displayRepo} ${parts[3].slice(0, 7)}`;
    description = item.commit.message;
  } else if (parts[2] === "actions" && parts[3] === "runs" && /^\d+$/.test(parts[4] ?? "")) {
    item = await github.read(repo, `/actions/runs/${parts[4]}`);
    title = `${displayRepo} ${item.name}`;
    description = `${item.status} · ${item.conclusion ?? ""}`;
  } else if (parts[2] === "releases" && parts[3] === "tag" && parts[4]) {
    item = await github.read(repo, `/releases/tags/${encode(parts.slice(4).join("/"))}`);
    title = `${displayRepo} ${item.name ?? item.tag_name}`;
    description = item.body ?? "";
  } else if (parts[2] === "blob" && parts.length >= 5) {
    const rest = parts.slice(3);
    let file: Json | undefined;
    // Longest matching ref also handles branch/tag names containing '/'.
    for (let split = Math.min(rest.length - 1, 12); split >= 1; split--) {
      try {
        file = await github.read(
          repo,
          `/contents/${rest.slice(split).map(encode).join("/")}?ref=${encode(rest.slice(0, split).join("/"))}`,
        );
        break;
      } catch (e) {
        if ((e as { status?: number }).status !== 404) throw e;
      }
    }
    if (!file || file.type !== "file" || file.encoding !== "base64" || file.size > 200000) return;
    let decoded: string;
    try {
      decoded = new TextDecoder("utf-8", { fatal: true }).decode(
        Buffer.from(String(file.content), "base64"),
      );
    } catch {
      return;
    }
    const lines = decoded.split("\n");
    if (lines.join("").includes("\u0000")) return;
    const range = /^#L(\d+)(?:-L(\d+))?$/.exec(u.hash);
    const start = Math.max(1, Number(range?.[1] ?? 1)),
      end = Math.min(lines.length, start + 19, Number(range?.[2] ?? start + 19));
    title = `${displayRepo}/${file.path} L${start}–L${end}`;
    description = lines.slice(start - 1, end).join("\n");
  } else if (parts.length > 2) return;
  return {
    url,
    card: {
      title: text(title, 500),
      description: text(description, 2000),
      url: u.toString(),
      source: "GitHub",
      author: text(item.user?.login ?? item.author?.login ?? metadata.owner?.login, 100),
      badge: text(badge, 100),
    },
  };
}
