// Use the consuming Playground's saved examples; keep their content in ignored output.
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve, join } from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("../../", import.meta.url));
const source = resolve(process.argv[2]);
const out = resolve(process.argv[3] ?? join(root, "output/rollout-documents"));
const require = createRequire(new URL("../../crates/md-to-docx-napi/index.js", import.meta.url));
const ts = require("typescript");
const syntax = ts.createSourceFile(source, readFileSync(source, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let basic, examples;
function visit(node) {
  if (ts.isVariableDeclaration(node) && node.name.getText(syntax) === "defaultMarkdown") basic = node.initializer.text;
  if (ts.isVariableDeclaration(node) && node.name.getText(syntax) === "examples") examples = node.initializer;
  ts.forEachChild(node, visit);
}
visit(syntax);
if (!basic || !examples) throw new Error("The source must contain the Playground's saved examples");
const cases = [{ name: "consumer-basic", markdown: basic, options: { documentType: "document", title: "MD to DOCX Playground" } }];
for (const property of examples.properties) if (property.initializer && (ts.isNoSubstitutionTemplateLiteral(property.initializer) || ts.isStringLiteral(property.initializer))) cases.push({ name: `consumer-${property.name.getText(syntax)}`, markdown: property.initializer.text, options: { documentType: property.name.getText(syntax) === "report" ? "report" : "document", title: property.initializer.text.match(/^# (.+)$/m)?.[1] } });
cases.push({ name: "project-readme", markdown: readFileSync(join(root, "README.md"), "utf8"), options: {} });
const report = cases.find(c => c.name === "consumer-report").markdown;
cases.push({ name: "reference-report", markdown: report, options: {}, kind: "reference" });
cases.push({ name: "patched-report", markdown: report, options: {}, kind: "patch", patches: { body: report, summary: "The saved Playground technical report.\n\n- Users increased 12.5%.\n- Revenue increased 8.2%." } });
const engines = { ts: await import(pathToFileURL(require.resolve("@mohtasham/md-to-docx")).href), rs: await import(new URL("../../crates/md-to-docx-napi/index.js", import.meta.url)) };
const reference = readFileSync(join(root, "crates/md-to-docx/tests/oracle/fixtures/reference-template.docx"));
mkdirSync(out, { recursive: true });
const index = [];
for (const c of cases) {
  writeFileSync(join(out, `${c.name}.md`), c.markdown);
  writeFileSync(join(out, `${c.name}.options.json`), JSON.stringify(c.options, null, 2));
  const entry = { name: c.name, source: c.name.startsWith("consumer") ? source : c.kind ? "checked-in reference fixture and saved consumer report" : "repository README", kind: c.kind ?? "convert" };
  for (const [engine, api] of Object.entries(engines)) {
    const bytes = c.kind === "reference" ? await api.convertMarkdownWithReferenceDocxToBuffer(c.markdown, reference, c.options)
      : c.kind === "patch" ? await api.patchMarkdownInDocxToBuffer(reference, c.patches, c.options)
      : await api.convertMarkdownToBuffer(c.markdown, c.options);
    writeFileSync(join(out, `${c.name}.${engine}.docx`), bytes);
    entry[engine] = { ok: true, size: bytes.length };
  }
  index.push(entry);
}
writeFileSync(join(out, "index.json"), JSON.stringify(index, null, 2));
console.log(`Generated ${cases.length * 2} DOCX files from ${cases.length} cases`);
