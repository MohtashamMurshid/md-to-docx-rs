// Build a platform-specific release candidate without changing the workspace package.
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
const root = fileURLToPath(new URL("../../", import.meta.url));
const version = process.argv[2];
if (!/^\d+\.\d+\.\d+-rc\.\d+$/.test(version)) throw new Error("Use an explicit x.y.z-rc.n version");
const destination = resolve(process.argv[3] ?? join(root, "output/release"));
const source = join(root, "crates/md-to-docx-napi");
const stage = join(destination, `stage-${process.platform}-${process.arch}`);
rmSync(stage, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
for (const name of ["index.js", "index.d.ts", "browser.js", "md-to-docx-napi.node"]) cpSync(join(source, name), join(stage, name));
cpSync(join(root, "LICENSE"), join(stage, "LICENSE"));
cpSync(join(root, "README.md"), join(stage, "README.md"));
const pkg = JSON.parse(readFileSync(join(source, "package.json")));
if (existsSync(join(destination, `${pkg.name.replace(/^@/, "").replace("/", "-")}-${version}.tgz`))) throw new Error("Candidate version already exists; choose a new RC version");
Object.assign(pkg, { version, os: [process.platform], cpu: [process.arch] });
delete pkg.devDependencies;
delete pkg.scripts;
pkg.files.push("README.md", "LICENSE");
// A dependency's overrides are ignored by its consumers. Bundle the installed
// compatibility tree so every package manager uses the versions we tested.
pkg.bundleDependencies = Object.keys(pkg.dependencies);
writeFileSync(join(stage, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
cpSync(join(source, "node_modules"), join(stage, "node_modules"), { recursive: true });
// Windows cannot execute npm.cmd without a shell. Invoke its JavaScript CLI
// with Node instead so destination paths never pass through shell expansion.
const npmArguments = ["pack", "--ignore-scripts", "--pack-destination", destination, "--json"];
const npmCli = process.env.npm_execpath ?? join(dirname(process.execPath), "node_modules/npm/bin/npm-cli.js");
const result = JSON.parse(execFileSync(process.platform === "win32" ? process.execPath : "npm", process.platform === "win32" ? [npmCli, ...npmArguments] : npmArguments, { cwd: stage, encoding: "utf8" }))[0];
const tarball = join(destination, result.filename);
const sha256 = bytes => createHash("sha256").update(bytes).digest("hex");
const report = { version, platform: process.platform, arch: process.arch, tarball, integrity: result.integrity, sha256: sha256(readFileSync(tarball)), binary_sha256: sha256(readFileSync(join(stage, "md-to-docx-napi.node"))), files: result.files.map(file => file.path) };
writeFileSync(join(destination, "candidate.json"), JSON.stringify(report, null, 2) + "\n");
console.log(JSON.stringify({ ...report, files: report.files.length }, null, 2));
