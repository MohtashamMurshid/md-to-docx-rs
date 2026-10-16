// Verify a packed candidate after installing it into an unrelated application.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { readFileSync, existsSync } from "node:fs";
import { pathToFileURL } from "node:url";

const app = createRequire(join(resolve(process.argv[2]), "package.json"));
const entry = app.resolve("md-to-docx-rs");
const native = createRequire(entry);
const compatibility = createRequire(native.resolve("@mohtasham/md-to-docx"));
function version(loader, name) {
  let directory = dirname(loader.resolve(name));
  while (!existsSync(join(directory, "package.json"))) directory = dirname(directory);
  return JSON.parse(readFileSync(join(directory, "package.json"))).version;
}
const versions = Object.fromEntries([
  ["md-to-docx-rs", version(app, "md-to-docx-rs")],
  ["@mohtasham/md-to-docx", version(native, "@mohtasham/md-to-docx")],
  ...["docx", "sharp", "katex"].map(name => [name, version(compatibility, name)]),
]);
for (const [name, expected] of Object.entries({ "@mohtasham/md-to-docx": "4.0.1", docx: "9.5.0", sharp: "0.35.5", katex: "0.18.2" })) assert.equal(versions[name], expected, name);
const api = await import(pathToFileURL(entry));
const buffer = await api.convertMarkdownToBuffer("# Installed candidate\n\n**Native** conversion.");
assert.ok(Buffer.isBuffer(buffer));
assert.equal(buffer.subarray(0, 2).toString(), "PK");
assert.ok(await api.convertMarkdownToDocx("Blob") instanceof Blob);
assert.ok(await api.convertMarkdownToArrayBuffer("ArrayBuffer") instanceof ArrayBuffer);
const template = await api.convertMarkdownToBuffer("{{body}}");
assert.ok((await api.patchMarkdownInDocxToBuffer(template, { body: "# One\n\n## Two" })).length);
assert.ok((await api.convertMarkdownWithReferenceDocxToBuffer("# Report", template)).length);
let pluginRan = false;
await api.convertMarkdownToBuffer("Plugin", { plugins: [{ apiVersion: 1, name: "release.smoke", transformAst() { pluginRan = true; } }] });
assert.ok(pluginRan, "bundled compatibility plugin did not run");
const highlighted = await api.convertMarkdownToBuffer("```ts\nconst value: number = 1;\n```", { codeHighlighting: { enabled: true } });
const { default: JSZip } = await import(pathToFileURL(compatibility.resolve("jszip")));
const highlightedXml = await (await JSZip.loadAsync(highlighted)).file("word/document.xml").async("string");
assert.match(highlightedXml, /<w:color w:val="D73A49"\/>[^]*?<w:t[^>]*>const<\/w:t>/, "installed highlighter did not color the const keyword");
const { default: sharp } = await import(pathToFileURL(compatibility.resolve("sharp")));
const png = await sharp({ create: { width: 2, height: 2, channels: 3, background: "white" } }).png().toBuffer();
await api.convertMarkdownToBuffer("![local](virtual:smoke)", { imageHandling: { resolve: async () => ({ data: png, contentType: "image/png" }) } });
await assert.rejects(api.convertMarkdownToBuffer("input", { maxInputLength: 1 }), error => typeof error.context.inputLength === "number");
const reason = { release: "cancel" };
await assert.rejects(api.convertMarkdownToBuffer("input", { signal: AbortSignal.abort(reason) }), error => error.context.reason === reason);
console.log(JSON.stringify({ node: process.version, platform: process.platform, arch: process.arch, versions, passed: true }, null, 2));
