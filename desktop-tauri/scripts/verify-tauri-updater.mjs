#!/usr/bin/env node
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

// ASN.1 DER SubjectPublicKeyInfo prefix for raw 32-byte Ed25519 keys (RFC 8410).
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

function decodeTextLines(raw, label) {
  const trimmed = raw.trim();
  assert.ok(trimmed.length > 0, `${label} must not be empty`);
  const decoded = trimmed.includes('\n')
    ? trimmed
    : Buffer.from(trimmed, 'base64').toString('utf8').trim();
  const lines = decoded.split(/\r?\n/);
  assert.ok(lines.length >= 2, `${label} must have at least 2 lines`);
  return lines;
}

export function parseMinisignPublicKey(pubkeyInput) {
  const lines = decodeTextLines(pubkeyInput, 'updater public key');
  const raw = Buffer.from(lines[1].trim(), 'base64');
  assert.equal(raw.length, 42, 'minisign public key payload must be 42 bytes');
  assert.equal(raw.subarray(0, 2).toString('ascii'), 'Ed', 'public key algorithm must be Ed');
  const keyId = raw.subarray(2, 10);
  const rawKey = raw.subarray(10, 42);
  const keyObject = crypto.createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, rawKey]),
    format: 'der',
    type: 'spki',
  });
  return {
    normalizedInput: pubkeyInput.trim(),
    keyId,
    keyObject,
  };
}

export function parseMinisignSignature(sigInput) {
  const lines = decodeTextLines(sigInput, 'updater signature');
  assert.ok(lines.length >= 4, 'minisign signature must have 4 lines');
  const raw = Buffer.from(lines[1].trim(), 'base64');
  assert.equal(raw.length, 74, 'minisign signature payload must be 74 bytes');
  const alg = raw.subarray(0, 2).toString('ascii');
  assert.ok(alg === 'ED' || alg === 'Ed', `unsupported minisign signature algorithm: ${alg}`);
  const keyId = raw.subarray(2, 10);
  const signature = raw.subarray(10, 74);
  const trustedPrefix = 'trusted comment: ';
  assert.ok(lines[2].startsWith(trustedPrefix), 'signature missing trusted comment line');
  const trustedBody = lines[2].slice(trustedPrefix.length);
  const globalSignature = Buffer.from(lines[3].trim(), 'base64');
  assert.equal(globalSignature.length, 64, 'global signature must be 64 bytes');
  return { alg, keyId, signature, trustedBody, globalSignature };
}

export async function verifyFileSignature(filePath, sigInput, pubkeyInput) {
  const pub = parseMinisignPublicKey(pubkeyInput);
  const sig = parseMinisignSignature(sigInput);
  assert.ok(pub.keyId.equals(sig.keyId), 'signature key ID does not match public key');

  let message;
  if (sig.alg === 'ED') {
    const hash = crypto.createHash('blake2b512');
    for await (const chunk of fs.createReadStream(filePath)) {
      hash.update(chunk);
    }
    message = hash.digest();
  } else {
    message = await fs.promises.readFile(filePath);
  }

  assert.ok(
    crypto.verify(null, message, pub.keyObject, sig.signature),
    `primary Ed25519 signature verification failed for ${filePath}`,
  );
  const globalMsg = Buffer.concat([sig.signature, Buffer.from(sig.trustedBody, 'utf8')]);
  assert.ok(
    crypto.verify(null, globalMsg, pub.keyObject, sig.globalSignature),
    `global Ed25519 signature verification failed for ${filePath}`,
  );
}

export function verifyEmbeddedPubkey(exePath, pubkeyInput) {
  const expected = pubkeyInput.trim();
  assert.ok(expected.length > 0, 'public key must not be empty');
  const data = fs.readFileSync(exePath);
  assert.ok(
    data.includes(Buffer.from(expected, 'utf8')),
    `embedded TAURI_UPDATER_PUBKEY not found in ${exePath}`,
  );
}

export async function verifyManifest({
  manifestPath,
  distDir,
  pubkey,
  requiredPlatforms = [],
  verifyFiles = false,
}) {
  const pub = parseMinisignPublicKey(pubkey);
  const manifest = JSON.parse(await fs.promises.readFile(manifestPath, 'utf8'));
  assert.equal(typeof manifest.version, 'string', 'manifest.version must be a string');
  assert.ok(manifest.version.length > 0, 'manifest.version must not be empty');
  assert.equal(typeof manifest.platforms, 'object', 'manifest.platforms must be an object');
  assert.ok(manifest.platforms !== null, 'manifest.platforms must not be null');

  const present = Object.keys(manifest.platforms);
  assert.ok(present.length > 0, 'manifest.platforms must not be empty');
  for (const required of requiredPlatforms) {
    assert.ok(required in manifest.platforms, `missing required platform in latest.json: ${required}`);
  }

  for (const [platform, entry] of Object.entries(manifest.platforms)) {
    assert.equal(typeof entry?.url, 'string', `${platform}: missing url`);
    assert.equal(typeof entry?.signature, 'string', `${platform}: missing signature`);
    const url = new URL(entry.url);
    const filename = path.posix.basename(url.pathname);
    assert.ok(filename.length > 0, `${platform}: invalid artifact url ${entry.url}`);
    assert.ok(
      entry.url.includes(manifest.version),
      `${platform}: artifact url ${entry.url} does not include version ${manifest.version}`,
    );

    const sig = parseMinisignSignature(entry.signature);
    assert.ok(
      pub.keyId.equals(sig.keyId),
      `${platform}: manifest signature key ID does not match public key`,
    );

    const sigFile = path.join(distDir, `${filename}.sig`);
    if (fs.existsSync(sigFile)) {
      const diskSig = (await fs.promises.readFile(sigFile, 'utf8')).trim();
      assert.equal(entry.signature.trim(), diskSig, `${platform}: signature mismatch vs ${sigFile}`);
    }

    if (verifyFiles) {
      const artifactFile = path.join(distDir, filename);
      assert.ok(fs.existsSync(artifactFile), `${platform}: artifact not found at ${artifactFile}`);
      await verifyFileSignature(artifactFile, entry.signature, pubkey);
    }
  }
  return manifest;
}

function parseArgs(argv) {
  const opts = { flags: new Set(), values: {}, positional: [] };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--verify-files') {
      opts.flags.add('verify-files');
    } else if (arg.startsWith('--')) {
      const val = argv[++i];
      assert.ok(val !== undefined, `missing value for ${arg}`);
      opts.values[arg.slice(2)] = val;
    } else {
      opts.positional.push(arg);
    }
  }
  return opts;
}

async function main(argv) {
  const opts = parseArgs(argv);
  if ('sig' in opts.values) {
    const filePath = opts.values.sig;
    const sigPath = opts.values['sig-file'] ?? opts.positional[0] ?? `${filePath}.sig`;
    const pubkey = opts.values.pubkey ?? opts.positional[1];
    assert.ok(pubkey, '--pubkey or positional <pubkey> is required with --sig');
    const sigContent = await fs.promises.readFile(sigPath, 'utf8');
    await verifyFileSignature(filePath, sigContent, pubkey);
    if (opts.values.exe) {
      verifyEmbeddedPubkey(opts.values.exe, pubkey);
    }
    console.log(`Verified signature: ${filePath}`);
    return;
  }

  if ('manifest' in opts.values) {
    const required = opts.values.require
      ? opts.values.require.split(',').map(s => s.trim()).filter(Boolean)
      : [];
    const manifest = await verifyManifest({
      manifestPath: opts.values.manifest,
      distDir: opts.values.dir ?? path.dirname(opts.values.manifest),
      pubkey: opts.values.pubkey,
      requiredPlatforms: required,
      verifyFiles: opts.flags.has('verify-files'),
    });
    console.log(
      `Verified latest.json (v${manifest.version}): ${Object.keys(manifest.platforms).join(', ')}`,
    );
    return;
  }

  throw new Error(
    'Usage: verify-tauri-updater.mjs --sig <file> --sig-file <sig> --pubkey <b64> [--exe <bin>] | --manifest <latest.json> --dir <dist> --pubkey <b64> [--require a,b] [--verify-files]',
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main(process.argv.slice(2));
}
