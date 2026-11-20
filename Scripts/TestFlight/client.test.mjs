import test from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync, verify } from 'node:crypto';
import { AppleClient, AppleAPIError, ConfigurationError, createToken } from './client.mjs';

const { privateKey, publicKey } = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
const credentials = {
  keyID: 'TESTKEY123', issuerID: 'test-issuer',
  privateKey: privateKey.export({ type: 'pkcs8', format: 'pem' }),
};

test('Apple向けJWTはES256で署名され、有効期間が5分に制限される', () => {
  const [header, payload, signature] = createToken(credentials, 1_000).split('.');
  assert.deepEqual(JSON.parse(Buffer.from(header, 'base64url').toString()), { alg: 'ES256', kid: 'TESTKEY123', typ: 'JWT' });
  assert.deepEqual(JSON.parse(Buffer.from(payload, 'base64url').toString()), {
    iss: 'test-issuer', iat: 1_000, exp: 1_300, aud: 'appstoreconnect-v1',
  });
  assert.ok(verify('sha256', Buffer.from(`${header}.${payload}`), {
    key: publicKey, dsaEncoding: 'ieee-p1363',
  }, Buffer.from(signature, 'base64url')));
});

test('個人APIキーはIssuer IDを使わずuserを指定する', () => {
  const payload = createToken({ ...credentials, issuerID: undefined }).split('.')[1];
  const claims = JSON.parse(Buffer.from(payload, 'base64url').toString());
  assert.equal(claims.sub, 'user');
  assert.equal(claims.iss, undefined);
});

test('認証情報がない場合はネットワークへ送信しない', async () => {
  const client = new AppleClient({}, { fetcher: () => assert.fail('通信してはいけません。') });
  await assert.rejects(client.request('/v1/apps'), ConfigurationError);
});

test('ページ送り先が別ホストの場合は認証情報を送信しない', async () => {
  let requests = 0;
  const client = new AppleClient(credentials, { fetcher: async () => {
    requests++;
    return Response.json({ data: [], links: { next: 'https://example.invalid/v1/users' } });
  } });
  await assert.rejects(client.list('/v1/users'), ConfigurationError);
  assert.equal(requests, 1);
});

test('エラー本文に含まれるメールアドレスを出力しない', async () => {
  const client = new AppleClient(credentials, { fetcher: async () => Response.json({
    errors: [{ detail: 'private@example.invalidを招待できません。' }],
  }, { status: 403 }) });
  await assert.rejects(client.request('/v1/users'), error => {
    assert.ok(error instanceof AppleAPIError);
    assert.equal(error.status, 403);
    assert.ok(!error.message.includes('private@example.invalid'));
    return true;
  });
});

test('一覧を最後のページまで取得し、リダイレクトを禁止する', async () => {
  let requests = 0;
  const client = new AppleClient(credentials, { fetcher: async (url, options) => {
    assert.equal(options.redirect, 'error');
    assert.match(new Headers(options.headers).get('Authorization'), /^Bearer /);
    requests++;
    return Response.json(requests === 1
      ? { data: [{ id: '1' }], links: { next: 'https://api.appstoreconnect.apple.com/v1/users?cursor=2' } }
      : { data: [{ id: '2' }] });
  } });
  assert.deepEqual(await client.list('/v1/users'), [{ id: '1' }, { id: '2' }]);
});
