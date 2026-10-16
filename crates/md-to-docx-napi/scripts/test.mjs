// Expand test filenames ourselves for Node 20 on Windows, without a shell.
import { readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const directory = new URL("../test/", import.meta.url);
const files = readdirSync(directory).filter(name => name.endsWith(".test.mjs"))
  .sort().map(name => fileURLToPath(new URL(name, directory)));
if (!files.length) throw new Error("No Node test files found");
const result = spawnSync(process.execPath, ["--test", ...files], { stdio: "inherit" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
