import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const requireFrontend = createRequire(new URL('../../frontend/package.json', import.meta.url));
const root = fileURLToPath(new URL('../../', import.meta.url));
export const loadTypeScript = () => requireFrontend('typescript');

export function parseHunks(diff) {
  return [...diff.matchAll(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/gm)].map(m => ({
    before: { start: Number(m[1]), count: m[2] === undefined ? 1 : Number(m[2]) },
    after: { start: Number(m[3]), count: m[4] === undefined ? 1 : Number(m[4]) },
  }));
}

function testRanges(source, ts) {
  const ast = ts.createSourceFile('test.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  if (ast.parseDiagnostics.length) return null;
  // Serial suites can intentionally pass state between cases.
  if (/\b(?:test\.describe|describe)\.(?:serial|sequential|configure)\s*\(/.test(source)) return null;
  const lines = source.split('\n');
  const ranges = [];
  const testCall = expression => {
    if (ts.isIdentifier(expression)) return ['test', 'it'].includes(expression.text);
    return ts.isPropertyAccessExpression(expression) && ['only', 'skip', 'fixme'].includes(expression.name.text) &&
      ts.isIdentifier(expression.expression) && ['test', 'it'].includes(expression.expression.text);
  };
  function visit(node) {
    if (ts.isCallExpression(node) && testCall(node.expression)) {
      const callback = node.arguments.at(-1);
      if (callback && (ts.isArrowFunction(callback) || ts.isFunctionExpression(callback))) {
        const title = node.arguments[0];
        const start = ast.getLineAndCharacterOfPosition(node.getStart(ast));
        const end = ast.getLineAndCharacterOfPosition(node.end);
        // Line-based diffs cannot distinguish two calls/shared code on one line.
        if (lines[start.line].slice(0, start.character).trim() || !/^[;\s]*$/.test(lines[end.line].slice(end.character))) return;
        ranges.push({
          start: start.line + 1,
          end: end.line + 1,
          title: title && (ts.isStringLiteral(title) || ts.isNoSubstitutionTemplateLiteral(title)) ? title.text : null,
        });
        return; // Runtime test.skip() calls inside a case are not new test cases.
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  return ranges;
}

// Only narrow cases when every edited line is inside a test declaration in both
// revisions. Imports, hooks, shared helpers, describe setup and removed cases
// fall back to the entire file/suite instead of guessing their dependencies.
export function selectChangedCases(before, after, hunks, ts, { requireTitle = true } = {}) {
  const oldCases = testRanges(before, ts);
  const newCases = testRanges(after, ts);
  if (!oldCases || !newCases || !hunks.length) return null;
  const selected = new Map();
  const containing = (cases, span) => {
    const matches = cases.filter(c => span.start >= c.start && span.start + span.count - 1 <= c.end);
    return matches.length === 1 ? matches[0] : null;
  };
  for (const hunk of hunks) {
    const oldCase = hunk.before.count ? containing(oldCases, hunk.before) : null;
    const newCase = hunk.after.count ? containing(newCases, hunk.after) : null;
    if ((hunk.before.count && !oldCase) || (hunk.after.count && !newCase)) return null;
    const target = newCase ?? (oldCase?.title && newCases.find(c => c.title === oldCase.title));
    if (!target || (requireTitle && target.title === null)) return null;
    // A removed test must not be mistaken for the next remaining test.
    if (oldCase && hunk.before.start <= oldCase.start && hunk.before.start + hunk.before.count - 1 >= oldCase.end && !newCase) return null;
    selected.set(target.start, target);
  }
  return selected.size ? [...selected.values()] : null;
}

export function readChangedCases(base, path, { git = execFileSync, compiler, requireTitle = true } = {}) {
  if (!/^[a-f0-9]{40}$/.test(base ?? '')) throw new Error('Invalid base SHA');
  const options = { cwd: root, encoding: 'utf8', timeout: 30000 };
  const before = git('git', ['show', `${base}:${path}`], options);
  const after = git('git', ['show', `HEAD:${path}`], options);
  const diff = git('git', ['diff', '--no-ext-diff', '--no-renames', '--unified=0', base, 'HEAD', '--', path], options);
  return selectChangedCases(before, after, parseHunks(diff), compiler ?? loadTypeScript(), { requireTitle });
}

export const escapePattern = value => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
