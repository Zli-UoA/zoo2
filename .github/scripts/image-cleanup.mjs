const SHA = '[a-f0-9]{40}';
const mainTag = new RegExp(`^${SHA}$`);
const previewTag = new RegExp(`^preview-${SHA}$`);
const prTag = new RegExp(`^pr-([1-9][0-9]*)-(${SHA})$`);
export const tagsOf = v => v.metadata?.container?.tags ?? [];

// A version can carry multiple tags. Delete only if EVERY tag is disposable.
// openPRs maps open PR numbers to head SHAs; the current head image is kept regardless of age.
export function candidates(versions, openPRs, {closedPR = null, now = Date.now()} = {}) {
  const sorted = [...versions].sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at) || b.id - a.id);
  const keep = new Set();
  const counts = new Map();
  for (const v of sorted) {
    const groups = new Set(tagsOf(v).flatMap(t => {
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
    return !keep.has(v.id) && tags.every(t => mainTag.test(t) || previewTag.test(t) || prTag.test(t));
  });
}

export async function ignoreMissing(operation) {
  try { return await operation(); } catch (error) { if (error.status === 404) return null; throw error; }
}

// Follow OCI indexes to protect untagged platform/attestation manifests as well.
export async function referencedDigests(roots, readManifest, {protectedDigests = new Set(), concurrency = 8} = {}) {
  async function visit(digest) {
    if (protectedDigests.has(digest)) return;
    protectedDigests.add(digest);
    const manifest = await readManifest(digest);
    await Promise.all((manifest.manifests ?? []).map(child => visit(child.digest)));
  }
  const queue = [...roots];
  await Promise.all(Array.from({length: Math.min(concurrency, queue.length)}, async () => {
    while (queue.length) await visit(queue.shift());
  }));
  return protectedDigests;
}

export default async function cleanup({github, context, core}) {
  const owner = context.repo.owner;
  const closedPR = context.eventName === 'pull_request_target' ? context.payload.pull_request.number : null;
  const dryRun = process.env.DRY_RUN !== 'false';
  let open;
  const listOpen = async () => new Map((await github.paginate(github.rest.pulls.list, {...context.repo, state: 'open', per_page: 100})).map(p => [p.number, p.head.sha]));
  // A PR tag must stay if its PR was reopened or it became the current head (e.g. force-push back).
  const stillNeeded = async (number, tagSha) => {
    const {data} = await github.rest.pulls.get({...context.repo, pull_number: number});
    return data.state === 'open' && (!open.has(number) || data.head.sha === tagSha);
  };
  // A running build may put an old untagged digest into a new index, so untagged deletion waits for it.
  const publishing = async () => {
    for (const status of ['in_progress', 'queued']) {
      const {data} = await github.rest.actions.listWorkflowRuns({...context.repo, workflow_id: 'cd-images.yaml', status, per_page: 1});
      if (data.total_count > 0) return true;
    }
    return false;
  };
  const rows = [];
  try {
    for (const component of ['frontend', 'backend', 'migrate']) {
      const package_name = `zoo2-${component}`;
      const params = {org: owner, package_type: 'container', package_name};
      const versions = await ignoreMissing(() => github.paginate(github.rest.packages.getAllPackageVersionsForPackageOwnedByOrg, {...params, per_page: 100}));
      if (versions === null) { core.notice(`${package_name}: not found`); continue; }
      open = await listOpen();
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
        // A missing manifest references nothing.
        if (response.status === 404) return {};
        if (!response.ok) throw new Error(`Cannot inspect protected manifest ${digest}: ${response.status}`);
        return response.json();
      };
      const protectedDigests = await referencedDigests(versions.filter(v => !selectedIDs.has(v.id)).map(v => v.name), readManifest);
      // A version kept after selection also keeps its children; skip it rather than abort the run.
      const skip = async (version, message) => {
        core.notice(`${package_name}/${version.id}: ${message}; keeping it and its referenced manifests`);
        await referencedDigests([version.name], readManifest, {protectedDigests});
      };
      let writersActive = null;
      // Delete parent indexes first. Abort on failure, leaving their children intact.
      selected.sort((a, b) => Number(tagsOf(b).length > 0) - Number(tagsOf(a).length > 0));
      for (const version of selected) {
        if (protectedDigests.has(version.name)) continue;
        const current = await ignoreMissing(() => github.rest.packages.getPackageVersionForPackageOwnedByOrg({...params, package_version_id: version.id}));
        if (!current) continue;
        if (JSON.stringify([...tagsOf(current.data)].sort()) !== JSON.stringify([...tagsOf(version)].sort())) {
          await skip(version, 'tags changed during cleanup');
          continue;
        }
        let needed = false;
        for (const [, number, tagSha] of tagsOf(version).map(t => t.match(prTag)).filter(Boolean)) {
          needed ||= await stillNeeded(Number(number), tagSha);
        }
        if (needed) {
          await skip(version, 'PR reopened or moved to this head during cleanup');
          continue;
        }
        // Untagged versions are sorted last.
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
    await core.summary.addHeading(dryRun ? 'GHCR cleanup dry run' : 'GHCR cleanup').addTable([
      [{data: 'Package', header: true}, {data: 'Version', header: true}, {data: 'Tags / digest', header: true}, {data: 'Reason', header: true}], ...rows,
    ]).write();
  }
}
