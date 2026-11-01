import { readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { storyNameFromExport } from "storybook/internal/csf";
import { expect, it } from "vite-plus/test";
import coverage from "../../../.storybook/coverage.json";
import { kotlinOptions, type Selection } from "../../../.storybook/kotlin-stories";

it("provides a real Kotlin implementation for every existing Storybook state", () => {
  const missing: string[] = [];
  for (const entry of coverage.stories) {
    const file = path.resolve(process.cwd(), entry.file);
    const ast = ts.createSourceFile(
      file,
      readFileSync(file, "utf8"),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
    let title = "";
    for (const node of ast.statements) {
      if (!ts.isVariableStatement(node)) continue;
      const meta = node.declarationList.declarations.find(
        (d) => ts.isIdentifier(d.name) && d.name.text === "meta",
      );
      let value = meta?.initializer;
      while (
        value &&
        (ts.isSatisfiesExpression(value) ||
          ts.isAsExpression(value) ||
          ts.isParenthesizedExpression(value))
      )
        value = value.expression;
      if (value && ts.isObjectLiteralExpression(value)) {
        const property = value.properties.find(
          (p) => ts.isPropertyAssignment(p) && p.name.getText(ast) === "title",
        );
        if (
          property &&
          ts.isPropertyAssignment(property) &&
          ts.isStringLiteral(property.initializer)
        )
          title = property.initializer.text;
      }
    }
    for (const node of ast.statements) {
      if (
        !ts.isVariableStatement(node) ||
        !node.modifiers?.some((m) => m.kind === ts.SyntaxKind.ExportKeyword)
      )
        continue;
      for (const declaration of node.declarationList.declarations) {
        if (!ts.isIdentifier(declaration.name)) continue;
        const name = storyNameFromExport(declaration.name.text);
        const context = {
          id: `${title}--${name}`,
          title,
          name,
          args: {},
          parameters: {},
          globals: { locale: "ja" },
        } as Selection;
        if (!kotlinOptions(context)) missing.push(`${title} / ${name}`);
      }
    }
  }
  expect(
    missing,
    "Every existing story must render its real Kotlin component; unavailable notices do not satisfy coverage",
  ).toEqual([]);
});
