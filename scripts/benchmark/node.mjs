// node --expose-gc node.mjs ENGINE INPUT OPTIONS OUTPUT WARMUPS RUNS
// Clock stops when the public API returns its Blob. Import/read/copy/write excluded.
import { readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { performance } from "node:perf_hooks";

const [engine, input, optionsPath, output, warmupsArg, runsArg] = process.argv.slice(2);
if (!["typescript", "rust-node"].includes(engine) || !runsArg) throw new Error("invalid benchmark arguments");
const warmups = Number(warmupsArg), runs = Number(runsArg);
if (!Number.isInteger(warmups) || warmups < 0 || !Number.isInteger(runs) || runs < 1) throw new Error("invalid run count");
const addonUrl = new URL("../../crates/md-to-docx-napi/index.js", import.meta.url);
const markdown = await readFile(input, "utf8");
const options = JSON.parse(await readFile(optionsPath, "utf8"));
let pluginCalls = 0;
if (options.benchmarkPlugin) {
  delete options.benchmarkPlugin;
  options.plugins = [{ apiVersion: 1, name: "benchmark.noop", transformAst() { pluginCalls++; } }];
}
const importStart = performance.now();
const moduleUrl = engine === "typescript"
  ? pathToFileURL(createRequire(addonUrl).resolve("@mohtasham/md-to-docx")) : addonUrl;
const { convertMarkdownToDocx } = await import(moduleUrl.href);
const import_ms = performance.now() - importStart;
const warmup_ms = [], samples_ms = [];
let result;
for (let i = 0; i < warmups + runs; i++) {
  result = undefined;
  // One collection before warmup establishes a common starting point.
  // No forced GC between conversions: allocation and normal GC are timed.
  if (i === 0) global.gc?.();
  const start = performance.now();
  result = await convertMarkdownToDocx(markdown, options);
  const elapsed = performance.now() - start;
  if (!(result instanceof Blob)) throw new Error("public API did not return a Blob");
  (i < warmups ? warmup_ms : samples_ms).push(elapsed);
}
if (options.plugins && pluginCalls !== warmups + runs) throw new Error("plugin did not run once per conversion");
const memory_at_return = process.memoryUsage();
await writeFile(output, Buffer.from(await result.arrayBuffer()));
console.log(JSON.stringify({ engine, input_bytes: Buffer.byteLength(markdown), input_chars: [...markdown].length,
  docx_bytes: result.size, import_ms, warmup_ms, samples_ms, memory_at_return, plugin_calls: pluginCalls }));
