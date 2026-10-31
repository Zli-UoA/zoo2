import { BotError, repoName, type Subscription, type Json } from "./types.ts";
export const FEATURES = [
  "issues",
  "pulls",
  "commits",
  "releases",
  "deployments",
  "workflows",
  "reviews",
  "comments",
  "branches",
  "tags",
  "discussions",
  "checks",
];
export const DEFAULTS = ["issues", "pulls", "commits", "releases", "deployments"];
export function words(input: string): string[] {
  const result: string[] = [];
  let current = "",
    quote = "",
    braces = 0;
  for (const c of input.trim()) {
    if (c === '"' || c === "'") {
      if (quote === c) {
        quote = "";
        if (braces) current += c;
      } else if (!quote) {
        quote = c;
        if (braces) current += c;
      } else current += c;
      continue;
    }
    if (!quote && c === "{") braces++;
    if (!quote && c === "}") braces--;
    if (!quote && !braces && /\s/.test(c)) {
      if (current) {
        result.push(current);
        current = "";
      }
    } else current += c;
    if (braces < 0) throw new BotError("フィルターの括弧が閉じていません。");
  }
  if (quote || braces) throw new BotError("引用符または括弧が閉じていません。");
  if (current) result.push(current);
  return result;
}
export function subscription(
  channel: string,
  repo: string,
  tokens: string[],
  old?: Subscription,
  remove = false,
): Subscription {
  const s: Subscription = structuredClone(
    old ?? {
      channel,
      repo: repoName(repo),
      features: remove
        ? []
        : tokens.some((t) => FEATURES.includes(t.split(":")[0]))
          ? []
          : [...DEFAULTS],
    },
  );
  for (const token of tokens) {
    if (token.startsWith("+label:")) {
      s.label = remove ? undefined : token.slice(7);
      if (s.label && s.label.length > 100) throw new BotError("ラベル名が長すぎます。");
      continue;
    }
    const [feature, ...rest] = token.split(":");
    if (!FEATURES.includes(feature)) throw new BotError(`不明な通知種別: ${feature}`);
    s.features = s.features.filter((f) => f !== feature);
    if (!remove) s.features.push(feature);
    if (feature === "commits") s.branch = remove ? undefined : rest.join(":") || undefined;
    if (feature === "comments") s.broadcastComments = !remove && rest.join(":") === "channel";
    if (feature === "reviews") s.broadcastReviews = !remove && rest.join(":") === "channel";
    if (feature === "workflows") {
      s.workflow = undefined;
      if (!remove && rest.length) {
        const raw = rest.join(":");
        if (!raw.startsWith("{") || !raw.endsWith("}"))
          throw new BotError('workflows:{name:"CI" branch:"main"} の形式で指定してください。');
        const filters: Record<string, string[]> = {};
        let consumed = "";
        const body = raw.slice(1, -1);
        const re =
          /(name|event|branch|actor):((?:"[^"]*"|'[^']*'|[^\s,]+)(?:,(?:"[^"]*"|'[^']*'|[^\s,]+))*)/g;
        let match: RegExpExecArray | null;
        let end = 0;
        while ((match = re.exec(body))) {
          if (body.slice(end, match.index).trim())
            throw new BotError("不明な workflow フィルターです。");
          filters[match[1]] = (match[2].match(/"[^"]*"|'[^']*'|[^,]+/g) ?? []).map((v) =>
            v.replace(/^["']|["']$/g, ""),
          );
          end = re.lastIndex;
          consumed += match[0];
        }
        if (!consumed || body.slice(end).trim())
          throw new BotError("不明な workflow フィルターです。");
        s.workflow = filters;
      }
    }
  }
  if (
    (s.branch && s.branch.length > 200) ||
    Object.values(s.workflow ?? {}).some(
      (values) => values.length > 20 || values.some((v) => !v || v.length > 200),
    )
  )
    throw new BotError("フィルターは各200文字・20候補以内で指定してください。");
  return s;
}
export function glob(pattern: string, value: string): boolean {
  let p = 0,
    v = 0,
    star = -1,
    mark = 0;
  while (v < value.length) {
    if (pattern[p] === value[v]) {
      p++;
      v++;
    } else if (pattern[p] === "*") {
      star = p++;
      mark = v;
    } else if (star >= 0) {
      p = star + 1;
      v = ++mark;
    } else return false;
  }
  while (pattern[p] === "*") p++;
  return p === pattern.length;
}
export function permits(s: Subscription, feature: string, payload: Json): boolean {
  if (!s.features.includes(feature)) return false;
  const item = payload.issue ?? payload.pull_request ?? payload.discussion;
  if (
    s.label &&
    ["issues", "pulls", "comments", "reviews"].includes(feature) &&
    !(item?.labels ?? []).some(
      (l: Json | string) => (typeof l === "string" ? l : l.name) === s.label,
    )
  )
    return false;
  if (feature === "commits")
    return (
      String(payload.ref ?? "").startsWith("refs/heads/") &&
      glob(
        s.branch ?? payload.repository.default_branch,
        String(payload.ref ?? "").replace(/^refs\/heads\//, ""),
      )
    );
  if (feature === "workflows") {
    const run = payload.workflow_run,
      filters = s.workflow;
    if (!filters)
      return (
        run.event === "pull_request" &&
        (run.pull_requests ?? []).some(
          (p: Json) => p.base?.ref === payload.repository.default_branch,
        )
      );
    const actual: Record<string, string> = {
      name: run.name,
      event: run.event,
      branch:
        run.event === "pull_request"
          ? (run.pull_requests?.[0]?.base?.ref ?? run.head_branch)
          : run.head_branch,
      actor: run.actor?.login ?? "",
    };
    return Object.entries(filters).every(([key, values]) =>
      values.some((v) => glob(v, actual[key] ?? "")),
    );
  }
  return true;
}
