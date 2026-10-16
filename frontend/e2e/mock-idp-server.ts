import { startMockIdp } from "./mock-idp.ts";

/**
 * 開発・E2E 用のモック IdP を単体起動する（外部依存なし）。
 *
 *   node e2e/mock-idp-server.ts
 *
 * Rust 側は `OIDC_ISSUER` / `OIDC_AUDIENCE` を出力された issuer / client に
 * 向ける。フロントは `VITE_OIDC_ISSUER` / `VITE_OIDC_CLIENT_ID` を合わせる。
 */
const port = Number(process.env.MOCK_IDP_PORT ?? "4000");
const clientId = process.env.MOCK_IDP_CLIENT_ID ?? "zoo";

const idp = await startMockIdp({ port, clientId });
console.log(`mock IdP listening: issuer=${idp.issuer} client_id=${idp.clientId}`);

const shutdown = async () => {
  await idp.close();
  process.exit(0);
};
process.on("SIGINT", () => void shutdown());
process.on("SIGTERM", () => void shutdown());
