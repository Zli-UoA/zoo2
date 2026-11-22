import test from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync } from 'node:crypto';
import { AppleClient, ConfigurationError } from './client.mjs';
import { appID, groupID, main, parseInvitees } from './manage.mjs';

const privateKey = generateKeyPairSync('ec', { namedCurve: 'prime256v1' }).privateKey
  .export({ type: 'pkcs8', format: 'pem' }).toString();
const person = { email: 'tester@example.invalid', firstName: '太郎', lastName: '試験' };
const baseEnv = {
  ASC_KEY_ID: 'TESTKEY123', ASC_ISSUER_ID: 'test-issuer', ASC_PRIVATE_KEY: privateKey,
  TESTFLIGHT_INVITEES_JSON: JSON.stringify([person]),
};

function fixture() {
  const state = {
    users: [], invitations: [], testers: [], members: [],
    roles: ['MARKETING'], visibleApps: [appID], internal: true, targetApp: appID,
    buildState: 'IN_BETA_TESTING', posts: [], logs: [], failEmail: '',
  };
  const fetcher = async (url, options) => {
    const path = url.pathname;
    if (options.method === 'POST') {
      const body = JSON.parse(options.body);
      if (body.data.attributes?.email === state.failEmail) return Response.json({}, { status: 503 });
      state.posts.push({ path, body });
      if (path === '/v1/userInvitations') {
        const invitation = { id: 'invitation', attributes: {
          ...body.data.attributes, expirationDate: new Date(Date.now() + 86_400_000).toISOString(),
        } };
        state.invitations.push(invitation);
        return Response.json({ data: invitation }, { status: 201 });
      }
      if (path === '/v1/betaTesters') {
        assert.deepEqual(body.data.relationships.betaGroups.data, [{ type: 'betaGroups', id: groupID }]);
        const tester = { id: 'new-tester', attributes: body.data.attributes };
        state.testers.push(tester);
        state.members.push(tester);
        return Response.json({ data: tester }, { status: 201 });
      }
      if (path === `/v1/betaGroups/${groupID}/relationships/betaTesters`) {
        state.members.push(state.testers.find(tester => tester.id === body.data[0].id));
        return new Response(null, { status: 204 });
      }
      assert.fail('未定義の更新APIが呼び出されました。');
    }
    if (path === `/v1/apps/${appID}`) return Response.json({ data: { attributes: { bundleId: 'works.zli.zoo' } } });
    if (path === `/v1/betaGroups/${groupID}`) return Response.json({ data: {
      attributes: { name: 'zli', isInternalGroup: state.internal },
      relationships: { app: { data: { id: state.targetApp } } },
    } });
    if (path.endsWith('/builds')) return Response.json({ data: [
      { id: 'build', attributes: { processingState: 'VALID', expired: false } },
    ] });
    if (path.endsWith('/buildBetaDetail')) return Response.json({ data: { attributes: { internalBuildState: state.buildState } } });
    if (path === '/v1/users') return Response.json({ data: state.users.filter(
      user => user.attributes.username === url.searchParams.get('filter[username]'),
    ) });
    if (path.endsWith('/visibleApps')) return Response.json({ data: state.visibleApps.map(id => ({ id })) });
    if (path === '/v1/userInvitations') return Response.json({ data: state.invitations });
    if (path === '/v1/betaTesters') return Response.json({ data: state.testers });
    if (path === `/v1/betaGroups/${groupID}/betaTesters`) return Response.json({ data: state.members });
    assert.fail('未定義の参照APIが呼び出されました。');
  };
  /** @param {string} operation @param {{email: string, firstName: string, lastName: string, accountEmail?: string}[]} people */
  const run = (operation, people = [person]) => main({
    ...baseEnv, TESTFLIGHT_OPERATION: operation, TESTFLIGHT_INVITEES_JSON: JSON.stringify(people),
  }, { fetcher, report: line => state.logs.push(line) });
  const accept = (username = person.email) => {
    state.invitations = [];
    state.users.push({ id: 'user', attributes: { username, roles: state.roles, allAppsVisible: false } });
  };
  return { state, run, accept };
}

test('招待はZoo2だけのマーケティング権限で1回だけ送り、承諾後にTestFlightへ追加する', async () => {
  const { state, run, accept } = fixture();
  assert.equal((await run('invite')).results[0].status, 'invited');
  assert.deepEqual(state.posts[0].body.data, {
    type: 'userInvitations', attributes: { ...person, roles: ['MARKETING'], allAppsVisible: false, provisioningAllowed: false },
    relationships: { visibleApps: { data: [{ type: 'apps', id: appID }] } },
  });
  assert.equal((await run('invite')).results[0].status, 'pending');
  assert.equal((await run('sync')).results[0].status, 'pending');
  assert.equal(state.posts.length, 1);
  accept();
  assert.equal((await run('sync')).results[0].status, 'added');
  assert.equal((await run('sync')).results[0].status, 'already');
  assert.deepEqual(state.posts.map(post => post.path), ['/v1/userInvitations', '/v1/betaTesters']);
  assert.ok(!state.logs.join('\n').includes(person.email));
  assert.ok(!state.logs.join('\n').includes(person.firstName));
  assert.ok(!state.logs.join('\n').includes(privateKey));
});

test('既存テスターは再作成せずグループへ追加する', async () => {
  const { state, run, accept } = fixture();
  accept();
  state.testers.push({ id: 'existing', attributes: { email: person.email } });
  await run('sync');
  await run('sync');
  assert.deepEqual(state.posts, [{ path: `/v1/betaGroups/${groupID}/relationships/betaTesters`, body: {
    data: [{ type: 'betaTesters', id: 'existing' }],
  } }]);
});

test('定期同期は未招待・期限切れのユーザーへ招待を送らない', async () => {
  const { state, run } = fixture();
  assert.equal((await run('sync')).results[0].status, 'needsInvite');
  state.invitations.push({ attributes: { email: person.email, expirationDate: '2000-01-01T00:00:00Z' } });
  assert.equal((await run('sync')).results[0].status, 'needsInvite');
  assert.equal(state.posts.length, 0);
});

test('予定確認は未招待・承諾済みのどちらでも更新しない', async () => {
  const { state, run, accept } = fixture();
  assert.equal((await run('plan')).results[0].status, 'plannedInvite');
  accept();
  assert.equal((await run('plan')).results[0].status, 'plannedAdd');
  assert.equal(state.posts.length, 0);
});

test('権限不足や別アプリの既存ユーザーにはアクセス権を追加しない', async () => {
  for (const scenario of ['role', 'app']) {
    const { state, run, accept } = fixture();
    if (scenario === 'role') state.roles = ['FINANCE'];
    else state.visibleApps = ['another-app'];
    accept();
    await assert.rejects(run('sync'), ConfigurationError);
    assert.equal(state.posts.length, 0);
    assert.match(state.logs.join('\n'), /権限・アプリへのアクセス/);
  }
});

test('対象が外部グループや別アプリなら招待前に停止する', async () => {
  for (const scenario of ['external', 'app']) {
    const { state, run } = fixture();
    if (scenario === 'external') state.internal = false;
    else state.targetApp = 'another-app';
    await assert.rejects(run('invite'), /一致しません/);
    assert.equal(state.posts.length, 0);
  }
});

test('利用可能なビルドがない場合は承諾済みユーザーの追加を待つ', async () => {
  const { state, run, accept } = fixture();
  state.buildState = 'EXPIRED';
  accept();
  assert.equal((await run('sync')).results[0].status, 'waitingBuild');
  assert.equal(state.posts.length, 0);
});

test('テスト開始前の配布可能なビルドにも最初のテスターを追加できる', async () => {
  const { state, run, accept } = fixture();
  state.buildState = 'READY_FOR_BETA_TESTING';
  accept();
  assert.equal((await run('sync')).results[0].status, 'added');
});

test('招待メールと異なるApple Accountで承諾した対象者を指定できる', async () => {
  const { state, run, accept } = fixture();
  const accountEmail = 'apple-account@example.invalid';
  accept(accountEmail);
  await run('sync', [{ ...person, accountEmail }]);
  assert.equal(state.posts[0].body.data.attributes.email, accountEmail);
});

test('途中でAPIが失敗しても、それまでに完了した招待は記録される', async () => {
  const { state, run } = fixture();
  state.failEmail = 'second@example.invalid';
  await assert.rejects(run('invite', [person, { ...person, email: state.failEmail }]));
  assert.equal(state.posts.length, 1);
  assert.match(state.logs.join('\n'), /対象1：App Store Connectへの招待を送信/);
  assert.ok(!state.logs.join('\n').includes('対象2：'));
});

test('名簿の不正な形式・重複を入力値を漏らさず拒否する', () => {
  const invalid = [
    'invalid', '[]', JSON.stringify([{}]), JSON.stringify([{ ...person, accountEmail: 123 }]),
    JSON.stringify([person, { ...person, email: person.email.toUpperCase() }]),
    JSON.stringify([{ ...person, email: 'invalid-address' }]),
    JSON.stringify([{ ...person, accountEmail: 'other@example.invalid' }, { ...person, email: 'other@example.invalid' }]),
  ];
  for (const input of invalid) assert.throws(() => parseInvitees(input), error => {
    assert.ok(error instanceof ConfigurationError);
    assert.ok(!error.message.includes(person.email));
    return true;
  });
  assert.equal(parseInvitees(JSON.stringify([person]))[0].accountEmail, person.email);
});

test('未接続のcheckは設定不足を明示し、inviteは失敗する', async () => {
  const logs = [];
  const options = { fetcher: async () => assert.fail('通信してはいけません。'), report: text => logs.push(text) };
  assert.deepEqual(await main({}, options), { configured: false });
  assert.match(logs[0], /未接続/);
  await assert.rejects(main({ TESTFLIGHT_OPERATION: 'invite' }, options), ConfigurationError);
});

test('参照専用クライアントはPOSTを通信前に拒否する', async () => {
  const client = new AppleClient({}, { fetcher: async () => assert.fail('通信してはいけません。') });
  await assert.rejects(client.request('/v1/userInvitations', { method: 'POST', body: {} }), ConfigurationError);
});
