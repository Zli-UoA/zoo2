#!/usr/bin/env node
// frontend が lucide-react から import しているアイコンを集め、
// 同じ版の lucide-static から SVG を取って desktop/assets/icons/ に置く。
// 集める対象は frontend/src の *.ts / *.tsx。*.stories.tsx、*.test.ts(x)、*.vrt.test.tsx、src/test/ は除く。
// 使い方: node desktop/scripts/collect-lucide-icons.mjs
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const frontend = join(root, 'frontend');
const outDir = join(root, 'desktop/assets/icons');

// アプリ本体が使うアイコンだけを集める。stories、テスト、VRT、src/test/ は除く。
const testDir = join(frontend, 'src/test');
export function isScanned(path) {
  if (!/\.(ts|tsx)$/.test(path)) return false;
  if (/\.(stories|test|vrt\.test)\.tsx?$/.test(path)) return false;
  return path !== testDir && !path.startsWith(testDir + sep);
}

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    if (e.isDirectory()) return e.name === 'node_modules' || p === testDir ? [] : walk(p);
    return isScanned(p) ? [p] : [];
  });
}

// import { A, B as C, type D } from "lucide-react"（複数行も含む）を読む。import type は無視する。
export function importedNames(source) {
  const names = [];
  for (const m of source.matchAll(/import\s+\{([^}]*)\}\s*from\s*["']lucide-react["']/g)) {
    for (const part of m[1].split(',')) {
      const spec = part.trim();
      if (!spec || spec.startsWith('type ')) continue;
      names.push(spec.split(/\s+as\s+/)[0].trim());
    }
  }
  return names;
}

// CloudOffIcon / LucideCloudOff / CloudOff -> cloud-off
export function toKebab(name) {
  const base = name.replace(/^Lucide(?=[A-Z])/, '').replace(/Icon$/, '');
  return base
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
    .replace(/([A-Za-z])([0-9])/g, '$1-$2')
    .replace(/([0-9])([A-Z])/g, '$1-$2')
    .toLowerCase();
}

function lockedVersion() {
  const lock = readFileSync(join(frontend, 'pnpm-lock.yaml'), 'utf8');
  const m = lock.match(/^ {2}lucide-react@(\d+\.\d+\.\d+):/m);
  if (!m) throw new Error('pnpm-lock.yaml に lucide-react が見つかりません');
  return m[1];
}

function main() {
  const version = lockedVersion();
  const all = new Set();
  for (const file of walk(join(frontend, 'src'))) {
    for (const n of importedNames(readFileSync(file, 'utf8'))) all.add(n);
  }
  const kebabs = [...new Set([...all].map(toKebab))].sort();

  const tmp = mkdtempSync(join(tmpdir(), 'lucide-static-'));
  try {
    const tgz = execFileSync('npm', ['pack', `lucide-static@${version}`, '--silent', '--pack-destination', tmp], { encoding: 'utf8' }).trim().split('\n').pop();
    execFileSync('tar', ['-xzf', join(tmp, tgz), '-C', tmp]);
    const src = join(tmp, 'package/icons');
    mkdirSync(outDir, { recursive: true });
    const missing = [];
    for (const k of kebabs) {
      const f = join(src, `${k}.svg`);
      if (!existsSync(f)) { missing.push(k); continue; }
      writeFileSync(join(outDir, `${k}.svg`), readFileSync(f));
    }
    // 使わなくなったアイコンは消す。
    for (const f of readdirSync(outDir)) {
      if (f.endsWith('.svg') && !kebabs.includes(f.slice(0, -4))) {
        rmSync(join(outDir, f));
        console.log(`削除: ${f}`);
      }
    }
    console.log(`lucide-static@${version}: ${kebabs.length - missing.length}/${kebabs.length} 個`);
    if (missing.length) {
      console.error(`見つからないアイコン: ${missing.join(', ')}`);
      process.exitCode = 1;
    }
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
