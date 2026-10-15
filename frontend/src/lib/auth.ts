export type TokenProvider = () => Promise<string | null> | string | null;

let tokenProvider: TokenProvider = () => null;

export function setTokenProvider(provider: TokenProvider): void {
  tokenProvider = provider;
}

export function resetTokenProvider(): void {
  tokenProvider = () => null;
}

export async function getToken(): Promise<string | null> {
  return await tokenProvider();
}
