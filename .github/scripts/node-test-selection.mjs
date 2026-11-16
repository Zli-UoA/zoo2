import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync, statSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve, relative } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { readChanges } from './test-selection-diff.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const requireBot = createRequire(new URL('../../apps/github-bot/package.json', import.meta.url));
const requireFrontend = createRequire(new URL('../../frontend/package.json', import.meta.url));

export function selectNodeTests(entries, changes, { compiler, read = path => readFileSync(resolve(root, path), 'utf8'), exists = path => statSync(resolve(root, path), { throwIfNoEntry: false })?.isFile() ?? false, frontendAliases = false, log = () => {}, onModule = () => {} } = {}) {
  if (!changes?.length || changes.some(c => !['A', 'M'].includes(c.status) || !/\.(ts|tsx|mjs|js)$/.test(c.path))) return entries;
  const changed = new Set(changes.map(c => c.path));
  const cache = new Map();
  const dependencies = path => {
    if (cache.has(path)) return cache.get(path);
    if (!/\.(?:[cm]?js|jsx|ts|tsx)$/.test(path)) return [];
    const source = read(path);
    const ast = compiler.createSourceFile(path, source, compiler.ScriptTarget.Latest, true);
    if (ast.parseDiagnostics.length) throw new Error('Cannot parse module');
    onModule(path, source);
    const imports = [];
    function visit(node) {
      if (compiler.isImportDeclaration(node) || compiler.isExportDeclaration(node)) {
        if (node.isTypeOnly || node.importClause?.isTypeOnly) return;
        const clause = node.importClause;
        if (clause && !clause.name && clause.namedBindings && compiler.isNamedImports(clause.namedBindings) && clause.namedBindings.elements.length > 0 && clause.namedBindings.elements.every(e => e.isTypeOnly)) return;
        if (node.moduleSpecifier) imports.push(node.moduleSpecifier.text);
      } else if (compiler.isCallExpression(node) && (node.expression.kind === compiler.SyntaxKind.ImportKeyword || node.expression.getText(ast) === 'require')) {
        if (!node.arguments[0] || !compiler.isStringLiteral(node.arguments[0])) throw new Error('Computed import');
        imports.push(node.arguments[0].text);
      } else if (compiler.isNewExpression(node) && node.expression.getText(ast) === 'URL' && node.arguments?.[1]?.getText(ast) === 'import.meta.url') {
        if (!compiler.isStringLiteral(node.arguments[0])) throw new Error('Computed resource URL');
        if (node.arguments[0].text.startsWith('.')) imports.push(node.arguments[0].text);
      }
      compiler.forEachChild(node, visit);
    }
    visit(ast);
    const paths = imports.flatMap(specifier => {
      // Vite's ?url/?raw/?worker imports still depend on the underlying file.
      specifier = specifier.split('?')[0];
      const fragment = specifier.indexOf('#', specifier.startsWith('#') ? 1 : 0);
      if (fragment >= 0) specifier = specifier.slice(0, fragment);
      let dependencyPath;
      if (frontendAliases && /^(#|@)\//.test(specifier)) dependencyPath = `frontend/src/${specifier.slice(2)}`;
      else {
        if (specifier.startsWith('#')) throw new Error('Unresolved package import');
        if (!specifier.startsWith('.')) return [];
        dependencyPath = relative(root, resolve(root, dirname(path), specifier)).replaceAll('\\', '/');
      }
      const target = [dependencyPath, `${dependencyPath}.ts`, `${dependencyPath}.tsx`, `${dependencyPath}.js`, `${dependencyPath}/index.ts`, `${dependencyPath}/index.tsx`, `${dependencyPath}/index.js`].find(exists);
      if (!target) throw new Error(`Unresolved module ${specifier}`);
      return [target];
    });
    cache.set(path, paths);
    return paths;
  };
  return entries.filter(entry => {
    try {
      const seen = new Set();
      const affected = path => {
        if (changed.has(path)) return true;
        if (seen.has(path)) return false;
        seen.add(path);
        return dependencies(path).some(affected);
      };
      return affected(entry);
    } catch (error) {
      log(`${entry}: ${error.message}`);
      return true; // Always run this entry if any of its dependencies are unknown.
    }
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const group = process.argv[2];
  if (!['bot', 'sdk', 'pwa', 'avatar'].includes(group)) throw new Error('Unsupported Node test group');
  const entries = group === 'bot'
    ? [...readdirSync(resolve(root, 'apps/github-bot/test')).filter(p => p.endsWith('.test.ts')).map(p => `apps/github-bot/test/${p}`), 'sdk/src/index.test.ts']
    : group === 'sdk' ? ['sdk/src/e2ee.test.ts', 'examples/e2ee-echo-bot/echo.test.ts']
      : [group === 'pwa' ? 'frontend/scripts/pwa.test.mjs' : 'frontend/scripts/avatar-cache.test.mjs'];
  let selected = entries;
  if (process.env.GITHUB_EVENT_NAME === 'pull_request') {
    try {
      const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
      const compiler = ['bot', 'sdk'].includes(group) ? requireBot('typescript') : requireFrontend('typescript');
      selected = selectNodeTests(entries, readChanges(event.pull_request?.base?.sha, execFileSync), { compiler, frontendAliases: ['pwa', 'avatar'].includes(group) });
    } catch { console.log('Node Test Selection: analysis unavailable; running all tests'); }
  }
  console.log(`Node Test Selection (${group}): ${selected.length}/${entries.length}: ${selected.join(', ')}`);
  if (selected.length) {
    const result = spawnSync('node', ['--experimental-strip-types', '--test', ...selected], { cwd: root, stdio: 'inherit' });
    process.exitCode = result.status ?? 1;
  }
}
