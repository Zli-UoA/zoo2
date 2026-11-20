import { createPrivateKey, sign } from 'node:crypto';

const origin = 'https://api.appstoreconnect.apple.com';

export class ConfigurationError extends Error {}

export class AppleAPIError extends Error {
  constructor(status) {
    super(`App Store Connect APIの処理に失敗しました（HTTP ${status}）。`);
    this.status = status;
  }
}

/** @param {{keyID?: string, issuerID?: string, privateKey?: string | Buffer}} credentials */
export function createToken({ keyID, issuerID, privateKey }, now = Math.floor(Date.now() / 1000)) {
  if (!keyID || !privateKey) {
    throw new ConfigurationError('ASC_KEY_IDとASC_PRIVATE_KEYをGitHub Secretsに登録してください。');
  }
  let key;
  try {
    key = createPrivateKey(privateKey);
  } catch {
    throw new ConfigurationError('ASC_PRIVATE_KEYをAppleから取得した秘密鍵の内容に設定してください。');
  }
  if (key.asymmetricKeyType !== 'ec' || key.asymmetricKeyDetails?.namedCurve !== 'prime256v1') {
    throw new ConfigurationError('ASC_PRIVATE_KEYにはES256用の秘密鍵が必要です。');
  }
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const header = encode({ alg: 'ES256', kid: keyID, typ: 'JWT' });
  const payload = encode({
    ...(issuerID ? { iss: issuerID } : { sub: 'user' }),
    iat: now, exp: now + 300, aud: 'appstoreconnect-v1',
  });
  const body = `${header}.${payload}`;
  const signature = sign('sha256', Buffer.from(body), { key, dsaEncoding: 'ieee-p1363' });
  return `${body}.${signature.toString('base64url')}`;
}

export class AppleClient {
  /** @param {Parameters<typeof createToken>[0]} credentials */
  constructor(credentials, { fetcher = fetch, allowWrites = false } = {}) {
    this.credentials = credentials;
    this.fetcher = fetcher;
    this.allowWrites = allowWrites;
  }

  /**
   * @param {string} path
   * @param {{method?: string, body?: unknown}} options
   */
  async request(path, { method = 'GET', body } = {}) {
    if (method !== 'GET' && !this.allowWrites) {
      throw new ConfigurationError('確認モードではApple側の情報を変更できません。');
    }
    const url = new URL(path, origin);
    // ページ送りのURLにも認証情報を渡すため、Apple以外への転送を拒否する。
    if (url.origin !== origin || !url.pathname.startsWith('/v1/') || url.username || url.password) {
      throw new ConfigurationError('App Store Connect以外のAPI URLは使用できません。');
    }
    const token = createToken(this.credentials);
    let response;
    try {
      response = await this.fetcher(url, {
        method,
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(30_000),
        redirect: 'error',
      });
    } catch {
      throw new Error('App Store Connect APIに接続できませんでした。時間をおいて再実行してください。');
    }
    // APIの応答本文には招待先の個人情報が含まれるため、エラーログには出さない。
    if (!response.ok) throw new AppleAPIError(response.status);
    if (response.status === 204) return undefined;
    try {
      return await response.json();
    } catch {
      throw new Error('App Store Connect APIの応答を読み取れませんでした。');
    }
  }

  async list(path) {
    const data = [];
    const visited = new Set();
    while (path) {
      if (visited.has(path) || visited.size >= 100) {
        throw new Error('App Store Connect APIの一覧取得が終了しませんでした。');
      }
      visited.add(path);
      const page = await this.request(path);
      if (!Array.isArray(page?.data)) throw new Error('App Store Connect APIの一覧形式が不正です。');
      data.push(...page.data);
      path = page.links?.next;
    }
    return data;
  }
}
