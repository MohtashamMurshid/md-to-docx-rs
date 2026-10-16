// Differential generator: converts every corpus case through TypeScript 4.0.1
// and the native addon and stores both outputs for compare.py.
import { mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { fork } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const outDir = process.argv[2] ?? join(root, "output", "parity");
const filter = process.argv[3] ? new RegExp(process.argv[3]) : null;

const cases = [];
const specDir = join(here, "corpus");
if (existsSync(join(specDir, "commonmark.json"))) {
  for (const ex of JSON.parse(readFileSync(join(specDir, "commonmark.json"), "utf8"))) {
    cases.push({ name: `cm-${String(ex.example).padStart(3, "0")}`, markdown: ex.markdown, options: undefined });
  }
}
if (existsSync(join(specDir, "gfm.json"))) {
  for (const ex of JSON.parse(readFileSync(join(specDir, "gfm.json"), "utf8"))) {
    cases.push({ name: `gfm-${String(ex.example).padStart(3, "0")}`, markdown: ex.markdown, options: undefined });
  }
}
for (const file of ["custom.mjs", "custom2.mjs"]) {
  const custom = (await import(join(specDir, file))).default;
  for (const c of custom) cases.push(c);
}

// Each engine lives in a restartable process. A blocking loop, OOM or native
// abort is recorded for that case without losing the rest of the corpus.
if (process.env.PARITY_ENGINE) {
  const engine = process.env.PARITY_ENGINE;
  const mod = await import(join(root, engine === "rs"
    ? "crates/md-to-docx-napi/index.js"
    : "crates/md-to-docx-napi/node_modules/@mohtasham/md-to-docx/dist/index.js"));
  process.on("message", async ({ name }) => {
    const c = cases.find(c => c.name === name);
    try {
      const opts = typeof c.options === "function" ? c.options() : c.options;
      let bytes;
      if (c.kind === "reference") bytes = await mod.convertMarkdownWithReferenceDocxToBuffer(c.markdown, c.reference(), opts);
      else if (c.kind === "patch") bytes = await mod.patchMarkdownInDocxToBuffer(c.reference(), c.patches, opts);
      else bytes = await mod.convertMarkdownToBuffer(c.markdown, opts);
      writeFileSync(join(outDir, `${c.name}.${engine}.docx`), bytes);
      process.send({ ok: true, size: bytes.length });
    } catch (error) {
      process.send({ ok: false, name: error?.name, message: String(error?.message ?? error), context: error?.context ?? null });
    }
  });
} else {
  mkdirSync(outDir, { recursive: true });
  const workers = new Map();
  function convert(engine, name) {
    let worker = workers.get(engine);
    if (!worker) {
      worker = fork(fileURLToPath(import.meta.url), [outDir], {
        env: { ...process.env, PARITY_ENGINE: engine },
        execArgv: ["--max-old-space-size=256"],
        stdio: ["ignore", "ignore", "pipe", "ipc"],
      });
      workers.set(engine, worker);
    }
    return new Promise(resolve => {
      let diagnostic = "";
      const stderr = chunk => { diagnostic = (diagnostic + chunk).slice(-4000); };
      const finish = result => {
        clearTimeout(timer);
        worker.off("message", message);
        worker.off("exit", exited);
        worker.stderr.off("data", stderr);
        resolve(result);
      };
      const message = result => finish(result);
      const exited = (code, signal) => {
        workers.delete(engine);
        finish({ ok: false, name: "ProcessError", message: `Conversion process exited: ${signal ?? code}`, context: { diagnostic } });
      };
      const timer = setTimeout(() => {
        workers.delete(engine);
        worker.kill("SIGKILL");
        finish({ ok: false, name: "TimeoutError", message: "Conversion exceeded 10000ms", context: { diagnostic } });
      }, 10000);
      worker.once("message", message);
      worker.once("exit", exited);
      worker.stderr.on("data", stderr);
      worker.send({ name });
    });
  }
  const index = [];
  let n = 0;
  for (const c of cases) {
    if (filter && !filter.test(c.name)) continue;
    if (process.env.PARITY_TRACE) console.error(`case ${c.name}`);
    const entry = { name: c.name, kind: c.kind ?? "convert", fixedTimestamps: ["created", "modified"].filter(key => typeof c.options !== "function" && c.options?.metadata?.[key] !== undefined) };
    for (const engine of ["ts", "rs"]) {
      if (process.env.PARITY_TRACE) console.error(`  ${engine}`);
      entry[engine] = await convert(engine, c.name);
    }
    index.push(entry);
    writeFileSync(join(outDir, "index.json"), JSON.stringify(index, null, 1));
    if (++n % 200 === 0) console.error(`${n} cases`);
  }
  writeFileSync(join(outDir, "index.json"), JSON.stringify(index, null, 1));
  console.error(`wrote ${index.length} cases to ${outDir}`);
  for (const worker of workers.values()) worker.kill();
}
