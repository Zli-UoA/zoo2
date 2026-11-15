export interface RefreshRequest {
  kind: "refresh";
  sessionId: string;
  tokenEndpoint: string;
  issuer: string;
  clientId: string;
  resource: string | null;
  refreshToken: string;
}

export type AuthWorkerRequest = RefreshRequest | { kind: "logout"; sessionId: string };
export type AuthTokenResponse = Record<string, unknown> & {
  access_token: string;
  refresh_token: string;
  expires_in: number;
};
export type AuthWorkerResponse =
  | { ok: true; body: AuthTokenResponse | null }
  | { ok: false; code: string; status?: number };
