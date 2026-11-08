import { readFileSync, readdirSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import ts from "typescript";

const root = fileURLToPath(new URL("../", import.meta.url));
const read = (file) => readFileSync(path.join(root, file), "utf8");
const catalog = JSON.parse(read(".storybook/coverage.json"));
const errors = [];
const check = (condition, message) => {
  if (!condition) errors.push(message);
};
function walk(dir) {
  return readdirSync(path.join(root, dir), { withFileTypes: true }).flatMap((entry) => {
    const file = `${dir}/${entry.name}`;
    return entry.isDirectory() ? walk(file) : [file];
  });
}
const files = walk("src");
const storyFiles = files.filter((file) => /\.stories\.tsx?$/.test(file));
const modules = files.filter(
  (file) =>
    file.startsWith("src/components/") &&
    file.endsWith(".tsx") &&
    !/\.(stories|test|browser\.test|vrt\.test)\.tsx$/.test(file),
);
const roots = new Set(["Foundations", "Components", "Features", "Pages", "Flows"]);
const titles = new Set();
let stateCount = 0;
const parsed = new Map();
for (const file of storyFiles) {
  const ast = ts.createSourceFile(
    file,
    read(file),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let title;
  const states = new Set();
  for (const node of ast.statements) {
    if (!ts.isVariableStatement(node)) continue;
    for (const declaration of node.declarationList.declarations) {
      if (!ts.isIdentifier(declaration.name)) continue;
      if (declaration.name.text === "meta") {
        let object = declaration.initializer;
        while (
          object &&
          (ts.isSatisfiesExpression(object) ||
            ts.isAsExpression(object) ||
            ts.isParenthesizedExpression(object))
        )
          object = object.expression;
        if (object && ts.isObjectLiteralExpression(object)) {
          const property = object.properties.find(
            (p) => ts.isPropertyAssignment(p) && p.name.getText(ast) === "title",
          );
          if (property && ts.isStringLiteral(property.initializer))
            title = property.initializer.text;
        }
      } else if (node.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword))
        states.add(declaration.name.text);
    }
  }
  check(
    title && roots.has(title.split("/")[0]),
    `${file}: title must start with an approved category`,
  );
  check(!titles.has(title), `${file}: duplicate title ${title}`);
  titles.add(title);
  check(states.size > 0, `${file}: no named stories`);
  stateCount += states.size;
  parsed.set(file, { title, states });
}
const registeredStories = new Set();
for (const entry of catalog.stories) {
  check(!registeredStories.has(entry.file), `duplicate story entry: ${entry.file}`);
  registeredStories.add(entry.file);
  const story = parsed.get(entry.file);
  check(!!story, `story file missing: ${entry.file}`);
  check(entry.states?.length > 0, `${entry.file}: declare required states`);
  for (const state of entry.states ?? [])
    check(story?.states.has(state), `${entry.file}: required state ${state} missing`);
}
for (const file of storyFiles) check(registeredStories.has(file), `unregistered story: ${file}`);
// Verify that a claimed story actually imports the UI module, directly or through composition.
const importCache = new Map();
function importsFrom(file) {
  if (importCache.has(file)) return importCache.get(file);
  const dependencies = [];
  const ast = ts.createSourceFile(
    file,
    read(file),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  for (const node of ast.statements) {
    if (!ts.isImportDeclaration(node) || !ts.isStringLiteral(node.moduleSpecifier)) continue;
    const specifier = node.moduleSpecifier.text;
    const base = specifier.startsWith(".")
      ? path.posix.join(path.posix.dirname(file), specifier)
      : specifier.startsWith("@/")
        ? `src/${specifier.slice(2)}`
        : null;
    if (!base) continue;
    const target = [
      base,
      `${base}.tsx`,
      `${base}.ts`,
      `${base}/index.tsx`,
      `${base}/index.ts`,
    ].find((candidate) => /\.tsx?$/.test(candidate) && existsSync(path.join(root, candidate)));
    if (target) dependencies.push(target);
  }
  importCache.set(file, dependencies);
  return dependencies;
}
function reaches(file, target, seen = new Set()) {
  if (file === target) return true;
  if (seen.has(file)) return false;
  seen.add(file);
  return importsFrom(file).some((dependency) => reaches(dependency, target, seen));
}
const registeredModules = new Set();
for (const entry of catalog.modules) {
  check(!registeredModules.has(entry.file), `duplicate module entry: ${entry.file}`);
  registeredModules.add(entry.file);
  check(modules.includes(entry.file), `UI module missing: ${entry.file}`);
  if (entry.story) {
    check(parsed.has(entry.story), `${entry.file}: coverage story missing: ${entry.story}`);
    if (parsed.has(entry.story))
      check(reaches(entry.story, entry.file), `${entry.file}: not imported by ${entry.story}`);
    if (entry.composed)
      check(entry.reason?.trim().length > 0, `${entry.file}: composed coverage needs a reason`);
  } else {
    check(
      entry.reason?.trim().length > 0 && entry.checks?.length > 0,
      `${entry.file}: exclusion needs reason and test references`,
    );
    for (const file of entry.checks ?? [])
      check(existsSync(path.join(root, file)), `${entry.file}: check missing: ${file}`);
  }
}
for (const file of modules) check(registeredModules.has(file), `unregistered UI module: ${file}`);
if (errors.length) {
  for (const error of errors) process.stderr.write(`${error}\n`);
  process.exitCode = 1;
} else {
  const excluded = catalog.modules.filter((entry) => !entry.story).length;
  process.stdout.write(
    `${storyFiles.length} story files, ${stateCount} states; ${modules.length - excluded} UI modules represented, ${excluded} explicit exclusions.\n`,
  );
  for (const category of roots)
    process.stdout.write(
      `  ${category}: ${[...parsed.values()].filter((entry) => entry.title?.startsWith(`${category}/`)).reduce((count, entry) => count + entry.states.size, 0)} states\n`,
    );
}
