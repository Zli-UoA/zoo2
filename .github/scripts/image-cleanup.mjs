const SHA = '[a-f0-9]{40}';
const mainTag = new RegExp(`^${SHA}$`);
const previewTag = new RegExp(`^preview-${SHA}$`);
const prTag = new RegExp(`^pr-([1-9][0-9]*)-(${SHA})$`);
export const tagsOf = v => v.metadata?.container?.tags ?? [];

// A version can carry multiple tags. Delete only if EVERY tag is disposable.
export function candidates(versions, openPRs, {closedPR = null, now = Date.now()} = {}) {
  const sorted = [...versions].sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at) || b.id - a.id);
  const keep = new Set();
  const counts = new Map();
  for (const v of sorted) {
    const groups = new Set(tagsOf(v).flatMap(t => {
      if (mainTag.test(t)) return ['main'];
      if (previewTag.test(t)) return ['preview'];
      const match = t.match(prTag);
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
    return !keep.has(v.id) && tags.every(t => mainTag.test(t) || previewTag.test(t) || prTag.test(t));
  });
}

export async function ignoreMissing(operation) {
  try { return await operation(); } catch (error) { if (error.status === 404) return null; throw error; }
}

// Follow OCI indexes to protect untagged platform/attestation manifests as well.
export async function referencedDigests(roots, readManifest) {
  const protectedDigests = new Set();
  async function visit(digest) {
    if (protectedDigests.has(digest)) return;
    protectedDigests.add(digest);
    const manifest = await readManifest(digest);
    for (const child of manifest.manifests ?? []) await visit(child.digest);
  }
  for (const digest of roots) await visit(digest);
  return protectedDigests;
}

export default async function cleanup({github, context, core}) {
  const owner = context.repo.owner;
  const closedPR = context.eventName === 'pull_request_target' ? context.payload.pull_request.number : null;
  const dryRun = process.env.DRY_RUN !== 'false';
  const listOpen = async () => new Set((await github.paginate(github.rest.pulls.list, {...context.repo, state: 'open', per_page: 100})).map(p => p.number));
  let open = await listOpen();
  const initialOpen = JSON.stringify([...open].sort((a, b) => a - b));
  const rows = [];
  for (const component of ['frontend', 'backend', 'migrate']) {
    const package_name = `zoo2-${component}`;
    const params = {org: owner, package_type: 'container', package_name};
    const versions = await ignoreMissing(() => github.paginate(github.rest.packages.getAllPackageVersionsForPackageOwnedByOrg, {...params, per_page: 100}));
    if (versions === null) { core.notice(`${package_name}: not found`); continue; }
    const selected = candidates(versions, open, {closedPR});
    if (!selected.length) continue;
    const selectedIDs = new Set(selected.map(v => v.id));
    const repository = `${owner.toLowerCase()}/${package_name}`;
    const auth = Buffer.from(`${process.env.GITHUB_ACTOR}:${process.env.GH_TOKEN}`).toString('base64');
    const tokenResponse = await fetch(`https://ghcr.io/token?service=ghcr.io&scope=${encodeURIComponent(`repository:${repository}:pull`)}`, {headers: {Authorization: `Basic ${auth}`}});
    if (!tokenResponse.ok) throw new Error(`GHCR token request: ${tokenResponse.status}`);
    const {token} = await tokenResponse.json();
    const readManifest = async digest => {
      const response = await fetch(`https://ghcr.io/v2/${repository}/manifests/${digest}`, {headers: {
        Authorization: `Bearer ${token}`,
        Accept: 'application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json',
      }});
      if (!response.ok) throw new Error(`Cannot inspect protected manifest ${digest}: ${response.status}`);
      return response.json();
    };
    const protectedDigests = await referencedDigests(versions.filter(v => !selectedIDs.has(v.id)).map(v => v.name), readManifest);
    // Delete parent indexes first. Abort on failure, leaving their children intact.
    selected.sort((a, b) => Number(tagsOf(b).length > 0) - Number(tagsOf(a).length > 0));
    for (const version of selected) {
      if (protectedDigests.has(version.name)) continue;
      open = await listOpen();
      if (JSON.stringify([...open].sort((a, b) => a - b)) !== initialOpen) {
        throw new Error('PR state changed during cleanup; rerun to recompute manifest protection');
      }
      const current = await ignoreMissing(() => github.rest.packages.getPackageVersionForPackageOwnedByOrg({...params, package_version_id: version.id}));
      if (!current) continue;
      if (JSON.stringify([...tagsOf(current.data)].sort()) !== JSON.stringify([...tagsOf(version)].sort())) {
        throw new Error(`Tags changed for ${version.id}; rerun to recompute manifest protection`);
      }
      if (!candidates(versions, open, {closedPR}).some(v => v.id === version.id)) continue;
      const reason = tagsOf(version).length ? (closedPR ? `closed PR ${closedPR}` : 'outside retained generations / closed PR') : 'untagged, older than 24 hours';
      core.info(`${dryRun ? 'Would delete' : 'Deleting'} ${package_name}/${version.id}: ${reason}`);
      rows.push([package_name, String(version.id), tagsOf(version).join(', ') || version.name, reason]);
      if (!dryRun) await ignoreMissing(() => github.rest.packages.deletePackageVersionForOrg({...params, package_version_id: version.id}));
    }
  }
  await core.summary.addHeading(dryRun ? 'GHCR cleanup dry run' : 'GHCR cleanup').addTable([
    [{data: 'Package', header: true}, {data: 'Version', header: true}, {data: 'Tags / digest', header: true}, {data: 'Reason', header: true}], ...rows,
  ]).write();
}
