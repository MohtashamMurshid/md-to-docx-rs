import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { test } from "node:test";

test("browser export resolves to the TypeScript implementation", async () => {
  const script = `
    import assert from "node:assert/strict";
    assert.ok(import.meta.resolve("md-to-docx-rs").endsWith("/browser.js"));
  `;
  const result = spawnSync(process.execPath, ["--conditions=browser", "--input-type=module", "-e", script], {
    cwd: new URL("..", import.meta.url), encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  // Import under Node's regular conditions: browser-conditioned dependencies
  // require a real DOM, which this test does not simulate.
  const browser = await import("../browser.js");
  const upstream = await import("@mohtasham/md-to-docx");
  assert.equal(browser.convertMarkdownToDocx, upstream.convertMarkdownToDocx);
  assert.equal(browser.downloadDocx, upstream.downloadDocx);
  assert.equal(typeof browser.parseToDocxOptions, "function");
});
