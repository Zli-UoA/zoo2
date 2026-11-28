import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve, relative } from 'node:path';
import { loadTypeScript, escapePattern } from './browser-test-selection.mjs';
import { selectNodeTests } from './node-test-selection.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));

function pureModule(source, ts) {
  const ast = ts.createSourceFile('source.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  if (ast.parseDiagnostics.length) return false;
  const pure = node => !node || ts.isLiteralExpression(node) || ts.isIdentifier(node) ||
    [ts.SyntaxKind.TrueKeyword, ts.SyntaxKind.FalseKeyword, ts.SyntaxKind.NullKeyword].includes(node.kind) ||
    ts.isArrowFunction(node) || ts.isFunctionExpression(node) ||
    ((ts.isAsExpression(node) || ts.isSatisfiesExpression(node) || ts.isParenthesizedExpression(node)) && pure(node.expression)) ||
    (ts.isArrayLiteralExpression(node) && node.elements.every(pure)) ||
    (ts.isObjectLiteralExpression(node) && node.properties.every(p =>
      !(p.name && ts.isComputedPropertyName(p.name)) && (ts.isPropertyAssignment(p) ? pure(p.initializer) : ts.isShorthandPropertyAssignment(p) || ts.isMethodDeclaration(p))));
  return ast.statements.every(s => ts.isImportDeclaration(s) || ts.isExportDeclaration(s) || ts.isFunctionDeclaration(s) ||
    ts.isInterfaceDeclaration(s) || ts.isTypeAliasDeclaration(s) || ts.isEmptyStatement(s) ||
    (ts.isExpressionStatement(s) && ts.isStringLiteral(s.expression)) ||
    (ts.isVariableStatement(s) && s.declarationList.declarations.every(d => pure(d.initializer))));
}

export function caseDependencies(source, ts) {
  const ast = ts.createSourceFile('test.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  if (ast.parseDiagnostics.length || /\b(?:test\.describe|describe)\.(?:serial|sequential|configure)\s*\(/.test(source)) return null;
  const imports = new Map(), declarations = new Map(), shared = [], cases = [];
  let aliasedRunner = false;
  function registerBinding(name, declaration) {
    if (ts.isIdentifier(name)) {
      const list = declarations.get(name.text) ?? [];
      list.push(declaration);
      declarations.set(name.text, list);
    } else {
      for (const element of name.elements) {
        if (ts.isBindingElement(element)) registerBinding(element.name, declaration);
      }
    }
  }
  function collect(node) {
    if (ts.isExpressionStatement(node) && !ts.isCallExpression(node.expression)) shared.push({ node });
    if (ts.isIfStatement(node) || ts.isSwitchStatement(node) || ts.isWhileStatement(node) || ts.isDoStatement(node)) shared.push({ node: node.expression });
    if (ts.isForStatement(node)) for (const expression of [node.initializer, node.condition, node.incrementor]) if (expression) shared.push({ node: expression });
    if (ts.isForOfStatement(node) || ts.isForInStatement(node)) shared.push({ node: node.expression });
    if (ts.isImportDeclaration(node)) {
      const spec = node.moduleSpecifier.text;
      const clause = node.importClause;
      if (clause?.isTypeOnly) return;
      if (!clause) shared.push({ module: spec });
      if (clause?.name) imports.set(clause.name.text, spec);
      if (clause?.namedBindings) {
        const bindings = clause.namedBindings;
        if (ts.isNamespaceImport(bindings)) imports.set(bindings.name.text, spec);
        else for (const binding of bindings.elements) if (!binding.isTypeOnly) {
          if (['it', 'test'].includes(binding.propertyName?.text) && binding.name.text !== binding.propertyName.text) aliasedRunner = true;
          imports.set(binding.name.text, spec);
        }
      }
      return;
    }
    if (ts.isCallExpression(node)) {
      const callee = node.expression.getText(ast);
      if (/^(test|it)(\.(only|skip|fixme))?$/.test(callee) && node.arguments.some(a => ts.isArrowFunction(a) || ts.isFunctionExpression(a))) {
        const title = node.arguments[0];
        let pattern;
        if (ts.isStringLiteral(title) || ts.isNoSubstitutionTemplateLiteral(title)) pattern = escapePattern(title.text);
        else if (ts.isTemplateExpression(title)) pattern = escapePattern(title.head.text) + title.templateSpans.map(s => `[\\s\\S]*${escapePattern(s.literal.text)}`).join('');
        else return cases.push({ unsupported: true });
        cases.push({ node, pattern: `(?:^| )${pattern}$` });
        return;
      }
      if (/^(test|it)\.(each|for|concurrent)|^(test|it)\.(each|for)\(/.test(callee)) { cases.push({ unsupported: true }); return; }
      if (/^(test|it)[.(]/.test(callee) && !/^(test\.describe)(\.(only|skip))?$/.test(callee) && node.arguments.some(a => ts.isArrowFunction(a) || ts.isFunctionExpression(a))) { cases.push({ unsupported: true }); return; }
      if (!/^(test\.describe|describe)(\.(only|skip))?$/.test(callee)) shared.push({ node });
    }
    if ((ts.isFunctionDeclaration(node) || ts.isVariableDeclaration(node) || ts.isClassDeclaration(node)) && node.name) {
      registerBinding(node.name, node);
      if (ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node)) return;
      if (ts.isVariableDeclaration(node) && node.initializer && (ts.isArrowFunction(node.initializer) || ts.isFunctionExpression(node.initializer))) return;
    }
    ts.forEachChild(node, collect);
  }
  collect(ast);
  if (aliasedRunner || !cases.length || cases.some(c => c.unsupported)) return null;
  let unknownImport = false;
  const dependencies = cases.map(c => {
    const modules = new Set(shared.flatMap(s => s.module ? [s.module] : []));
    const visited = new Set();
    function references(node) {
      if (visited.has(node)) return;
      visited.add(node);
      if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
        const specifier = node.arguments[0];
        if (specifier && (ts.isStringLiteral(specifier) || ts.isNoSubstitutionTemplateLiteral(specifier))) {
          modules.add(specifier.text);
        } else {
          unknownImport = true;
        }
      }
      if (ts.isIdentifier(node)) {
        if (imports.has(node.text)) modules.add(imports.get(node.text));
        for (const declaration of declarations.get(node.text) ?? []) references(declaration);
      }
      ts.forEachChild(node, references);
    }
    references(c.node);
    for (const item of shared) if (item.node) references(item.node);
    return { pattern: c.pattern, modules: [...modules] };
  });
  return unknownImport ? null : dependencies;
}

export function selectSourceCases(project, selection, changes, base, { compiler, git = execFileSync, read = path => readFileSync(resolve(root, path), 'utf8'), exists = path => statSync(resolve(root, path), { throwIfNoEntry: false })?.isFile() ?? false, files, log = () => {} } = {}) {
  if (!['unit', 'browser', 'vrt'].includes(project) || selection.mode !== 'related' || !changes?.every(c => c.status === 'M' && /^frontend\/src\/.+\.tsx?$/.test(c.path) && !/\.test\.tsx?$/.test(c.path))) return selection;
  try {
    const ts = compiler ?? loadTypeScript();
    for (const { path } of changes) {
      const before = git('git', ['show', `${base}:${path}`], { cwd: root, encoding: 'utf8', timeout: 30000 });
      const after = read(path);
      const importLines = source => source.match(/\b(?:import|export)\s[^;]*?\bfrom\s*["'][^"']+["']|\bimport\s*["'][^"']+["']/g)?.join('\n') ?? '';
      if (!pureModule(before, ts) || !pureModule(after, ts) || importLines(before) !== importLines(after)) { log(`Case dependencies: module initialization/imports require file-level tests (${path})`); return selection; }
    }
    const candidates = files ?? readdirSync(resolve(root, 'frontend/src'), { recursive: true, withFileTypes: true })
      .filter(entry => entry.isFile()).map(entry => relative(root, resolve(entry.parentPath, entry.name)).replaceAll('\\', '/'));
    const matches = path => {
      if (project === 'vrt') return /\.vrt\.test\.tsx$/.test(path);
      if (project === 'browser') return /\.browser\.test\.tsx?$/.test(path);
      return /(?<!\.browser)\.test\.ts$/.test(path);
    };
    const virtual = new Map(), patterns = new Map(), fullFiles = [];
    for (const path of candidates.filter(matches)) {
      const relevant = selectNodeTests([path], changes, { compiler: ts, read, exists, frontendAliases: true });
      if (!relevant.length) continue;
      const cases = caseDependencies(read(path), ts);
      if (!cases) {
        log(`Case dependencies: unsupported test declarations; running file in full (${path})`);
        fullFiles.push(path.slice('frontend/'.length));
        continue;
      }
      // All imported modules execute at file load, even if a particular case
      // does not reference their bindings. Initialization with possible side
      // effects is therefore a dependency of every case, including transitively.
      const effects = new Set();
      let unknown = false;
      selectNodeTests([path], [{ status: 'M', path: '__dependency_scan_only__.ts' }], {
        compiler: ts, read, exists, frontendAliases: true,
        log: () => { unknown = true; },
        onModule: (modulePath, source) => {
          if (modulePath !== path && !pureModule(source, ts)) effects.add(modulePath);
        },
      });
      if (unknown) {
        fullFiles.push(path.slice('frontend/'.length));
        continue;
      }
      cases.forEach((c, index) => {
        const entry = `${dirname(path)}/__selected_case_${index}_${virtual.size}.ts`;
        const effectImports = [...effects].map(modulePath => {
          const spec = relative(dirname(path), modulePath).replaceAll('\\', '/');
          return spec.startsWith('.') ? spec : `./${spec}`;
        });
        virtual.set(entry, [...c.modules, ...effectImports].map(module => `import ${JSON.stringify(module)};`).join('\n'));
        patterns.set(entry, c.pattern);
      });
    }
    const selected = selectNodeTests([...virtual.keys()], changes, {
      compiler: ts, frontendAliases: true,
      read: path => virtual.has(path) ? virtual.get(path) : read(path),
      exists: path => virtual.has(path) || exists(path),
    });
    return { ...selection, patterns: [...new Set(selected.map(path => patterns.get(path)))], fullFiles };
  } catch (error) { log(`Case dependencies: analysis unavailable (${error.message}); retaining file-level tests`); return selection; }
}
