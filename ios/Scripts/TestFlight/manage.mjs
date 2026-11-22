import { appendFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { AppleClient, AppleAPIError, ConfigurationError } from './client.mjs';

export const appID = '6836791230';
export const groupID = '23cb9feb-5ccb-4a4b-829e-11a9b6f4fd0f';
const groupPath = `/v1/betaGroups/${groupID}`;
const eligibleRoles = new Set(['ACCOUNT_HOLDER', 'ADMIN', 'APP_MANAGER', 'DEVELOPER', 'MARKETING']);
const normalize = value => value.trim().toLowerCase();
const query = (path, filters) => `${path}?${new URLSearchParams({ ...filters, limit: '200' })}`;

export function parseInvitees(text) {
  let rows;
  try { rows = JSON.parse(text); } catch {
    throw new ConfigurationError('TESTFLIGHT_INVITEES_JSONには対象者のJSON配列を設定してください。');
  }
  if (!Array.isArray(rows) || rows.length === 0 || rows.length > 100) {
    throw new ConfigurationError('招待対象者は1〜100人で指定してください。');
  }
  const seen = new Set();
  return rows.map((row, index) => {
    if (!row || ['email', 'firstName', 'lastName'].some(key => typeof row[key] !== 'string' || !row[key].trim())) {
      throw new ConfigurationError(`対象${index + 1}のemail・firstName・lastNameを指定してください。`);
    }
    const email = normalize(row.email);
    if (row.accountEmail != null && typeof row.accountEmail !== 'string') {
      throw new ConfigurationError(`対象${index + 1}のaccountEmailはメールアドレスで指定してください。`);
    }
    const accountEmail = normalize(row.accountEmail ?? email);
    if ([email, accountEmail].some(value => !/^[^\s@,]+@[^\s@,]+\.[^\s@,]+$/.test(value))) {
      throw new ConfigurationError(`対象${index + 1}のメールアドレス形式が不正です。`);
    }
    if (seen.has(email) || seen.has(accountEmail)) {
      throw new ConfigurationError(`対象${index + 1}が別の対象者と重複しています。`);
    }
    seen.add(email);
    seen.add(accountEmail);
    return { email, accountEmail, firstName: row.firstName.trim(), lastName: row.lastName.trim() };
  });
}

export async function inspectTarget(client) {
  const app = await client.request(`/v1/apps/${appID}`);
  const group = await client.request(`${groupPath}?include=app`);
  if (app?.data?.attributes?.bundleId !== 'works.zli.zoo'
      || group?.data?.attributes?.name !== 'zli'
      || group.data.attributes.isInternalGroup !== true
      || group.data.relationships?.app?.data?.id !== appID) {
    throw new ConfigurationError('対象アプリまたは内部グループがZoo2のzliと一致しません。');
  }
  const builds = await client.list(`${groupPath}/builds?limit=200`);
  let testing = false;
  for (const build of builds) {
    if (build.attributes?.expired || build.attributes?.processingState !== 'VALID') continue;
    const detail = await client.request(`/v1/builds/${encodeURIComponent(build.id)}/buildBetaDetail`);
    if (['IN_BETA_TESTING', 'READY_FOR_BETA_TESTING'].includes(detail?.data?.attributes?.internalBuildState)) {
      testing = true;
      break;
    }
  }
  return { testing };
}

const labels = {
  invited: 'App Store Connectへの招待を送信',
  pending: 'App Store Connectへの参加承諾待ち',
  needsInvite: '招待の送信または承諾アカウントの確認が必要',
  added: '内部グループへ追加',
  already: '内部グループへ登録済み',
  waitingBuild: 'テスト可能なビルド待ち',
  blocked: '既存ユーザーの権限・アプリへのアクセスの確認が必要',
  plannedInvite: 'App Store Connectへの招待予定',
  plannedAdd: '内部グループへの追加予定',
};

export async function manageInvitees(client, invitees, operation, target, onResult = result => {}) {
  if (!['plan', 'invite', 'sync'].includes(operation)) throw new ConfigurationError('操作の指定が不正です。');
  const members = await client.list(`${groupPath}/betaTesters?limit=200`);
  const memberEmails = new Set(members.map(row => row.attributes?.email).filter(Boolean).map(normalize));
  const results = [];
  for (const [index, person] of invitees.entries()) {
    let status;
    const users = await client.list(query('/v1/users', { 'filter[username]': person.accountEmail }));
    const user = users.find(row => normalize(row.attributes?.username ?? '') === person.accountEmail);
    if (!user) {
      const invitations = await client.list(query('/v1/userInvitations', { 'filter[email]': person.email }));
      const pending = invitations.some(row => normalize(row.attributes?.email ?? '') === person.email
        && Date.parse(row.attributes?.expirationDate) > Date.now());
      if (pending) status = 'pending';
      else if (operation === 'plan') status = 'plannedInvite';
      else if (operation === 'sync') status = 'needsInvite';
      else {
        await client.request('/v1/userInvitations', { method: 'POST', body: { data: {
          type: 'userInvitations',
          attributes: {
            email: person.email, firstName: person.firstName, lastName: person.lastName,
            roles: ['MARKETING'], allAppsVisible: false, provisioningAllowed: false,
          },
          relationships: { visibleApps: { data: [{ type: 'apps', id: appID }] } },
        } } });
        status = 'invited';
      }
    } else {
      const eligible = user.attributes.roles?.some(role => eligibleRoles.has(role));
      const visibleApps = user.attributes.allAppsVisible ? []
        : await client.list(`/v1/users/${encodeURIComponent(user.id)}/visibleApps?limit=200`);
      if (!eligible || (!user.attributes.allAppsVisible && !visibleApps.some(app => app.id === appID))) {
        status = 'blocked';
      } else if (memberEmails.has(person.accountEmail)) status = 'already';
      else if (!target.testing) status = 'waitingBuild';
      else if (operation === 'plan') status = 'plannedAdd';
      else {
        const testers = await client.list(query('/v1/betaTesters', { 'filter[email]': person.accountEmail }));
        const tester = testers.find(row => normalize(row.attributes?.email ?? '') === person.accountEmail);
        if (tester) {
          await client.request(`${groupPath}/relationships/betaTesters`, { method: 'POST', body: {
            data: [{ type: 'betaTesters', id: tester.id }],
          } });
        } else {
          await client.request('/v1/betaTesters', { method: 'POST', body: { data: {
            type: 'betaTesters',
            attributes: { email: person.accountEmail, firstName: person.firstName, lastName: person.lastName },
            relationships: { betaGroups: { data: [{ type: 'betaGroups', id: groupID }] } },
          } } });
        }
        memberEmails.add(person.accountEmail);
        status = 'added';
      }
    }
    const result = { index: index + 1, status };
    results.push(result);
    // 途中で失敗しても、送信済みの招待を管理者が確認できるようにする。
    onResult(result);
  }
  return results;
}

export async function main(env = process.env, { fetcher = fetch, report = console.log } = {}) {
  const write = text => {
    report(text);
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, `${text}\n`);
  };
  const operation = env.TESTFLIGHT_OPERATION || 'check';
  if (!['check', 'plan', 'invite', 'sync'].includes(operation)) throw new ConfigurationError('操作の指定が不正です。');
  const missing = ['ASC_KEY_ID', 'ASC_PRIVATE_KEY'].filter(name => !env[name]);
  if (missing.length) {
    write(`未接続：GitHub Secretsの${missing.join('・')}が未登録です。Appleへの招待・変更は実行していません。`);
    if (operation !== 'check') throw new ConfigurationError('Apple APIの認証情報を設定してから実行してください。');
    return { configured: false };
  }
  const people = operation === 'check' ? [] : parseInvitees(env.TESTFLIGHT_INVITEES_JSON);
  const client = new AppleClient({
    keyID: env.ASC_KEY_ID, issuerID: env.ASC_ISSUER_ID, privateKey: env.ASC_PRIVATE_KEY,
  }, { fetcher, allowWrites: operation === 'invite' || operation === 'sync' });
  const target = await inspectTarget(client);
  write(`Zoo2の内部グループzliへの接続を確認しました。テスト可能なビルド：${target.testing ? 'あり' : 'なし'}。`);
  if (operation === 'check') {
    write(`招待名簿：${env.TESTFLIGHT_INVITEES_JSON ? '設定あり（形式はplanで確認）' : '未登録'}。`);
    return { configured: true };
  }
  if (operation === 'plan') write('確認のみ：招待・グループ追加は実行しません。');
  const results = await manageInvitees(client, people, operation, target,
    result => write(`対象${result.index}：${labels[result.status]}。`));
  write('招待の承諾とTestFlightでの参加・インストールは本人の操作が必要です。');
  if (results.some(result => result.status === 'blocked')) {
    throw new ConfigurationError('権限またはアプリへのアクセスを確認する必要がある対象者がいます。');
  }
  return { configured: true, results };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => {
    // 想定外の例外にもHTTP本文や入力値が含まれうるため、詳細は公開ログに出さない。
    const message = error instanceof ConfigurationError || error instanceof AppleAPIError ? error.message
      : '処理に失敗しました。APIキーの権限・Apple側の状態を確認し、再実行してください。';
    console.error(message);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${message}\n`);
    process.exitCode = 1;
  });
}
