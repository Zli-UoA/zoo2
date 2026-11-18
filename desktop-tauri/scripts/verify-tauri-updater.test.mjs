import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import {
  verifyEmbeddedPubkey,
  verifyFileSignature,
  verifyManifest,
} from './verify-tauri-updater.mjs';

const ED25519_SPKI_HEADER_LEN = 12;

function encodeEd25519Pair() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  const spki = publicKey.export({ format: 'der', type: 'spki' });
  const rawPub = spki.subarray(ED25519_SPKI_HEADER_LEN);
  const keyId = crypto.randomBytes(8);
  const pubPayload = Buffer.concat([Buffer.from('Ed', 'ascii'), keyId, rawPub]).toString('base64');
  const pubText = `untrusted comment: minisign public key: ${keyId.toString('hex').toUpperCase()}\n${pubPayload}\n`;
  return {
    keyId,
    privateKey,
    pubkeyB64: Buffer.from(pubText, 'utf8').toString('base64'),
  };
}

function signPrehashed({ keyId, privateKey }, filename, content) {
  const digest = crypto.createHash('blake2b512').update(content).digest();
  const signature = crypto.sign(null, digest, privateKey);
  const sigLine = Buffer.concat([Buffer.from('ED', 'ascii'), keyId, signature]).toString('base64');
  const trustedBody = `timestamp:1795000000\tfile:${filename}`;
  const globalSig = crypto
    .sign(null, Buffer.concat([signature, Buffer.from(trustedBody, 'utf8')]), privateKey)
    .toString('base64');
  const sigText = [
    'untrusted comment: signature from tauri secret key',
    sigLine,
    `trusted comment: ${trustedBody}`,
    globalSig,
  ].join('\n');
  return Buffer.from(`${sigText}\n`, 'utf8').toString('base64');
}

test('verifyManifest validates windows-x86_64 installer signature and rejects tampering', async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'zoo2-updater-verify-'));
  try {
    const keys = encodeEd25519Pair();
    const setupName = 'zoo2_0.2.12_x64-setup.exe';
    const setupPath = path.join(tmp, setupName);
    const payload = Buffer.from('MZ-fake-inno-setup-installer');
    fs.writeFileSync(setupPath, payload);

    const sigB64 = signPrehashed(keys, setupName, payload);
    fs.writeFileSync(`${setupPath}.sig`, sigB64);

    const manifestPath = path.join(tmp, 'latest.json');
    fs.writeFileSync(
      manifestPath,
      JSON.stringify({
        version: '0.2.12',
        notes: 'https://github.com/Zli-UoA/zoo2/releases/tag/desktop-tauri-v0.2.12',
        pub_date: '2026-11-19T00:00:00Z',
        platforms: {
          'windows-x86_64': {
            url: `https://github.com/Zli-UoA/zoo2/releases/download/desktop-tauri-v0.2.12/${setupName}`,
            signature: sigB64,
          },
        },
      }),
    );

    const verified = await verifyManifest({
      manifestPath,
      distDir: tmp,
      pubkey: keys.pubkeyB64,
      requiredPlatforms: ['windows-x86_64'],
      verifyFiles: true,
    });
    assert.equal(verified.version, '0.2.12');

    await assert.rejects(
      verifyManifest({
        manifestPath,
        distDir: tmp,
        pubkey: keys.pubkeyB64,
        requiredPlatforms: ['windows-x86_64', 'darwin-aarch64'],
        verifyFiles: true,
      }),
      /missing required platform/,
    );

    fs.writeFileSync(setupPath, Buffer.from('MZ-tampered-installer'));
    await assert.rejects(
      verifyFileSignature(setupPath, sigB64, keys.pubkeyB64),
      /signature verification failed/,
    );

    const fakeExe = path.join(tmp, 'zoo2-desktop-tauri.exe');
    fs.writeFileSync(fakeExe, Buffer.concat([Buffer.from('MZ...'), Buffer.from(keys.pubkeyB64)]));
    verifyEmbeddedPubkey(fakeExe, keys.pubkeyB64);

    const wrongExe = path.join(tmp, 'zoo2-wrong.exe');
    fs.writeFileSync(wrongExe, Buffer.from('MZ-no-pubkey'));
    assert.throws(() => verifyEmbeddedPubkey(wrongExe, keys.pubkeyB64), /not found/);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});
