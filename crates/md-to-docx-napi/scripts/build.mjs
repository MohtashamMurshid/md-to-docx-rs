import { spawnSync } from "node:child_process";
import { copyFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(here, "../..");
const result = spawnSync("cargo", ["build", "--locked", "--release", "-p", "md-to-docx-napi"], {
  cwd: workspace,
  stdio: "inherit",
});
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const target = process.env.CARGO_TARGET_DIR
  ? resolve(workspace, process.env.CARGO_TARGET_DIR)
  : join(workspace, "target");
const library = process.platform === "win32"
  ? "md_to_docx_napi.dll"
  : process.platform === "darwin"
    ? "libmd_to_docx_napi.dylib"
    : "libmd_to_docx_napi.so";
copyFileSync(join(target, "release", library), join(here, "md-to-docx-napi.node"));
