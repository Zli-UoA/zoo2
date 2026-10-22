// GHCR の古いイメージを回収する。cd-image-cleanup.yaml から、PR の close 時と日次で呼ぶ。
// DRY_RUN=false のときだけ実際に削除し、それ以外は削除対象をサマリーに出すだけにする。

// 回収対象にするタグの形式。main は `<SHA>`、Preview 用 frontend は `preview-<SHA>`、PR は `pr-<番号>-<SHA>`。
// main の昇格で `<SHA>` と同じバージョンに付くリリースタグ `0.0.<実行番号>` は、その `<SHA>` の世代と一緒に回収する。
// latest・preview などそれ以外のタグが付いたバージョンは削除しない。
const SHA = '[a-f0-9]{40}';
const mainTag = new RegExp(`^${SHA}$`);
const releaseTag = /^0\.0\.[1-9][0-9]*$/;
const previewTag = new RegExp(`^preview-${SHA}$`);
const prTag = new RegExp(`^pr-([1-9][0-9]*)-(${SHA})$`);
export const tagsOf = v => v.metadata?.container?.tags ?? [];

// 削除候補のバージョンを返す。1つのバージョンには複数のタグが付くことがあり、すべてのタグが削除してよいものの場合だけ候補にする。
// openPRs は open な PR の番号から head SHA への Map。PR の現在の head と main の head のイメージは、世代数にかかわらず残す。
// - main・preview: 新しい順に15世代を残す
// - open な PR: PR ごとに新しい順に2世代を残す
// - タグなし: 作成から24時間を過ぎたものを候補にする
// - closedPR を指定した場合（PR close 時）: その PR のタグだけが付いたバージョンを候補にする。再 open 済みなら残す
export function candidates(versions, openPRs, {closedPR = null, mainHead = null, now = Date.now()} = {}) {
  // 新しい順に並べ、同じ作成時刻なら ID の大きい方を新しいとみなす。
  const sorted = [...versions].sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at) || b.id - a.id);
  const keep = new Set();
  const counts = new Map();
  for (const v of sorted) {
    const groups = new Set(tagsOf(v).flatMap(t => {
      if (mainHead && (t === mainHead || t === `preview-${mainHead}`)) keep.add(v.id);
      if (mainTag.test(t)) return ['main'];
      if (previewTag.test(t)) return ['preview'];
      const match = t.match(prTag);
      if (match && openPRs.get(Number(match[1])) === match[2]) keep.add(v.id);
      return match && openPRs.has(Number(match[1])) ? [`pr-${match[1]}`] : [];
    }));
    for (const group of groups) {
      const count = (counts.get(group) ?? 0) + 1;
      counts.set(group, count);
      if (count <= (group.startsWith('pr-') ? 2 : 15)) keep.add(v.id);
    }
  }
  return sorted.filter(v => {
    const tags = tagsOf(v);
    if (!tags.length) return now - Date.parse(v.created_at) >= 24 * 60 * 60 * 1000;
    if (closedPR !== null) return !openPRs.has(closedPR) && tags.every(t => Number(t.match(prTag)?.[1]) === closedPR);
    return !keep.has(v.id) && tags.every(t => mainTag.test(t) || releaseTag.test(t) || previewTag.test(t) || prTag.test(t));
  });
}

// 404 を「すでに存在しない」とみなして null を返す。他のエラーはそのまま投げる。
export async function ignoreMissing(operation) {
  try { return await operation(); } catch (error) { if (error.status === 404) return null; throw error; }
}

// 残すイメージから参照されている digest をすべて集める。
// OCI の index をたどり、タグのない各プラットフォームのマニフェストや attestation も保護する。
// index の幅や深さにかかわらず、同時に読み込むマニフェストは `concurrency` 件までにする。
export async function referencedDigests(roots, readManifest, {protectedDigests = new Set(), concurrency = 8} = {}) {
  let active = 0;
  const waiting = [];
  const limited = async digest => {
    if (active >= concurrency) await new Promise(resolve => waiting.push(resolve));
    active++;
    try { return await readManifest(digest); } finally { active--; waiting.shift()?.(); }
  };
  async function visit(digest) {
    if (protectedDigests.has(digest)) return;
    protectedDigests.add(digest);
    const manifest = await limited(digest);
    await Promise.all((manifest.manifests ?? []).map(child => visit(child.digest)));
  }
  await Promise.all(roots.map(visit));
  return protectedDigests;
}

// 回収の本体。パッケージ（frontend・backend・migrate）ごとに削除候補を選び、削除の直前に状態を確認し直してから削除する。
export default async function cleanup({github, context, core}) {
  const owner = context.repo.owner;
  const closedPR = context.eventName === 'pull_request_target' ? context.payload.pull_request.number : null;
  const dryRun = process.env.DRY_RUN !== 'false';
  const listOpen = async () => new Map((await github.paginate(github.rest.pulls.list, {...context.repo, state: 'open', per_page: 100})).map(p => [p.number, p.head.sha]));
  // PR が再 open された場合や、そのタグの SHA が再び head になった場合（古い SHA への force-push など）は、PR のタグを残す。
  const stillNeeded = async (number, tagSha) => {
    const {data} = await github.rest.pulls.get({...context.repo, pull_number: number});
    return data.state === 'open' && (!open.has(number) || data.head.sha === tagSha);
  };
  let open;
  const getMainHead = async () => (await github.rest.repos.getBranch({...context.repo, branch: 'main'})).data.commit.sha;
  // 実行中のビルドは、タグのない古い digest を新しい index に含めることがある。そのため、ビルド中はタグなしの削除を待つ。
  const publishing = async () => {
    // concurrency の待ち行列に入った実行は、queued ではなく pending・waiting・requested として返る。
    for (const status of ['in_progress', 'queued', 'pending', 'waiting', 'requested']) {
      const {data} = await github.rest.actions.listWorkflowRuns({...context.repo, workflow_id: 'cd-images.yaml', status, per_page: 1});
      if (data.total_count > 0) return true;
    }
    return false;
  };
  const rows = [];
  try {
    // open な PR と main の head は最初に一度だけ取得する。各バージョンは削除の直前に確認し直す。
    open = await listOpen();
    const mainHead = await getMainHead();
    for (const component of ['frontend', 'backend', 'migrate']) {
      const package_name = `zoo2-${component}`;
      const params = {org: owner, package_type: 'container', package_name};
      const versions = await ignoreMissing(() => github.paginate(github.rest.packages.getAllPackageVersionsForPackageOwnedByOrg, {...params, per_page: 100}));
      if (versions === null) { core.notice(`${package_name}: not found`); continue; }
      const selected = candidates(versions, open, {closedPR, mainHead});
      if (!selected.length) continue;
      const selectedIDs = new Set(selected.map(v => v.id));
      // マニフェストの参照関係を読むため、GHCR の Registry API を pull 権限のトークンで呼ぶ。
      const repository = `${owner.toLowerCase()}/${package_name}`;
      const auth = Buffer.from(`${process.env.GITHUB_ACTOR}:${process.env.GH_TOKEN}`).toString('base64');
      const fetchToken = async () => {
        const response = await fetch(`https://ghcr.io/token?service=ghcr.io&scope=${encodeURIComponent(`repository:${repository}:pull`)}`, {headers: {Authorization: `Basic ${auth}`}});
        if (!response.ok) throw new Error(`GHCR token request: ${response.status}`);
        return (await response.json()).token;
      };
      let token = await fetchToken();
      const requestManifest = digest => fetch(`https://ghcr.io/v2/${repository}/manifests/${digest}`, {headers: {
        Authorization: `Bearer ${token}`,
        Accept: 'application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json',
      }});
      const readManifest = async digest => {
        let response = await requestManifest(digest);
        // 削除のループが長いと、途中で Registry のトークンが失効することがある。その場合は一度だけ取り直す。
        if (response.status === 401) {
          token = await fetchToken();
          response = await requestManifest(digest);
        }
        // 存在しないマニフェストは何も参照していないものとして扱う。
        if (response.status === 404) return {};
        if (!response.ok) throw new Error(`Cannot inspect protected manifest ${digest}: ${response.status}`);
        return response.json();
      };
      // 削除候補にならなかったバージョンと、それらが参照する digest は削除しない。
      const protectedDigests = await referencedDigests(versions.filter(v => !selectedIDs.has(v.id)).map(v => v.name), readManifest);
      // 候補に選んだ後に残すと決めたバージョンは、参照先も一緒に残す。実行全体は中断せず、そのバージョンだけを飛ばす。
      const skip = async (version, message) => {
        core.notice(`${package_name}/${version.id}: ${message}; keeping it and its referenced manifests`);
        await referencedDigests([version.name], readManifest, {protectedDigests});
      };
      let writersActive = null;
      // 親の index（タグ付き）を先に削除する。失敗したらそこで中断し、参照先のマニフェストは残したままにする。
      selected.sort((a, b) => Number(tagsOf(b).length > 0) - Number(tagsOf(a).length > 0));
      for (const version of selected) {
        if (protectedDigests.has(version.name)) continue;
        // 一覧の取得後に削除済み、またはタグが変わったバージョンは削除しない。
        const current = await ignoreMissing(() => github.rest.packages.getPackageVersionForOrganization({...params, package_version_id: version.id}));
        if (!current) continue;
        if (JSON.stringify([...tagsOf(current.data)].sort()) !== JSON.stringify([...tagsOf(version)].sort())) {
          await skip(version, 'tags changed during cleanup');
          continue;
        }
        // 一覧の取得後に、PR が再 open されたり、main がこのコミットに戻ったりしていないかを確認する。
        let needed = false;
        for (const [, number, tagSha] of tagsOf(version).map(t => t.match(prTag)).filter(Boolean)) {
          needed ||= await stillNeeded(Number(number), tagSha);
        }
        if (needed) {
          await skip(version, 'PR reopened or moved to this head during cleanup');
          continue;
        }
        const mainShas = tagsOf(version).map(t => t.match(mainTag)?.[0] ?? t.match(previewTag)?.[0].slice('preview-'.length)).filter(Boolean);
        if (mainShas.length && mainShas.includes(await getMainHead())) {
          await skip(version, 'main moved to this commit during cleanup');
          continue;
        }
        // タグなしのバージョンは最後に並んでいる。イメージのビルド中なら、ここから先は次回の実行に回す。
        if (!tagsOf(version).length && (writersActive ??= await publishing())) {
          core.notice(`${package_name}: image publication is running; deferring untagged versions to the next run`);
          break;
        }
        const reason = tagsOf(version).length ? (closedPR ? `closed PR ${closedPR}` : 'outside retained generations / closed PR') : 'untagged, older than 24 hours';
        core.info(`${dryRun ? 'Would delete' : 'Deleting'} ${package_name}/${version.id}: ${reason}`);
        if (!dryRun) await ignoreMissing(() => github.rest.packages.deletePackageVersionForOrg({...params, package_version_id: version.id}));
        rows.push([package_name, String(version.id), tagsOf(version).join(', ') || version.name, reason]);
      }
    }
  } finally {
    // 途中で失敗しても、それまでに削除した（dry run では削除予定の）バージョンをサマリーに残す。
    await core.summary.addHeading(dryRun ? 'GHCR cleanup dry run' : 'GHCR cleanup').addTable([
      [{data: 'Package', header: true}, {data: 'Version', header: true}, {data: 'Tags / digest', header: true}, {data: 'Reason', header: true}], ...rows,
    ]).write();
  }
}
