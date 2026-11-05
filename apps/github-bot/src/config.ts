import { createPrivateKey } from "node:crypto";
import { readFileSync } from "node:fs";
import { repoName, type Config, type Rule } from "./types.ts";
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const required = (key: string) => {
    const value = env[key];
    if (!value) throw new Error(`Set ${key}`);
    return value;
  };
  const file = JSON.parse(readFileSync(required("BOT_CONFIG"), "utf8"));
  if (!Array.isArray(file.repositories) || !file.repositories.length)
    throw new Error("Configure repository/channel permissions in BOT_CONFIG");
  const rules: Rule[] = file.repositories.map((r: any) => {
    if (
      !Number.isSafeInteger(r.installationId) ||
      r.installationId <= 0 ||
      !Array.isArray(r.channels) ||
      !r.channels.length ||
      r.channels.some((c: unknown) => typeof c !== "string" || !c || c.length > 128)
    )
      throw new Error("Invalid repository/channel rule");
    return {
      repository: repoName(r.repository),
      installationId: r.installationId,
      channels: [...new Set<string>(r.channels)],
    };
  });
  // Conflicting installation policies would make webhook routing ambiguous.
  for (const [i, r] of rules.entries())
    for (const other of rules.slice(i + 1))
      if (
        (r.repository === other.repository ||
          (r.repository.endsWith("/*") && other.repository.startsWith(r.repository.slice(0, -1))) ||
          (other.repository.endsWith("/*") &&
            r.repository.startsWith(other.repository.slice(0, -1)))) &&
        r.installationId !== other.installationId
      )
        throw new Error("Overlapping repository rules must use the same installation");
  const zooUrl = new URL(required("ZOO_URL"));
  if (
    zooUrl.protocol !== "https:" &&
    !(zooUrl.protocol === "http:" && ["localhost", "127.0.0.1"].includes(zooUrl.hostname))
  )
    throw new Error("ZOO_URL must use HTTPS (localhost allowed for development)");
  if (zooUrl.username || zooUrl.password || zooUrl.search || zooUrl.hash || zooUrl.pathname !== "/")
    throw new Error("ZOO_URL must be an origin");
  const key = Buffer.from(required("BOT_ENCRYPTION_KEY"), "base64");
  if (key.length !== 32) throw new Error("BOT_ENCRYPTION_KEY must encode 32 random bytes");
  const port = Number(env.PORT ?? 3001);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("Invalid PORT");
  const users: Record<string, string> = {};
  for (const [login, user] of Object.entries(file.users ?? {})) {
    if (!/^[a-z0-9-]{1,39}$/i.test(login) || typeof user !== "string" || !user || user.length > 128)
      throw new Error("Invalid GitHub/Zoo mention mapping");
    users[login.toLowerCase()] = user;
  }
  const privateKey = readFileSync(required("GITHUB_PRIVATE_KEY_FILE"), "utf8");
  if (createPrivateKey(privateKey).asymmetricKeyType !== "rsa")
    throw new Error("GitHub App needs an RSA private key");
  return {
    rules,
    users,
    zooUrl: zooUrl.origin,
    appId: required("GITHUB_APP_ID"),
    privateKey,
    githubSecret: required("GITHUB_WEBHOOK_SECRET"),
    zooSecret: required("ZOO_SIGNING_SECRET"),
    encryptionKey: key,
    database: env.BOT_DATABASE ?? "data/github-bot.sqlite",
    port,
    host: env.HOST ?? "0.0.0.0",
  };
}
