import assert from "node:assert/strict";
import { test } from "node:test";
import * as native from "../index.js";
import * as reference from "@mohtasham/md-to-docx";
import first from "../../../scripts/parity/corpus/custom.mjs";
import second from "../../../scripts/parity/corpus/custom2.mjs";

const selection = /^(x-(images-strict-alt|images-placeholder-alt|captions-throw|deep-nesting|max-input-length|max-elements)|y-err-|y-replacements-(too-many|long-pattern|long-text)$|y-metadata-full-core$|y-images-alt-title-invalid$|y-patch-limits$)/;
async function result(api, c) {
  try {
    const options = typeof c.options === "function" ? c.options() : c.options;
    if (c.kind === "patch") await api.patchMarkdownInDocxToBuffer(c.reference(), c.patches, options);
    else await api.convertMarkdownToBuffer(c.markdown, options);
    return { ok: true };
  } catch (error) {
    if (error.context?.originalError !== undefined) assert.ok(error.context.originalError instanceof Error);
    return { ok: false, name: error.name, message: error.message, context: JSON.parse(JSON.stringify(error.context ?? null)) };
  }
}

test("81 corpus cases preserve error names, messages and typed context values", async () => {
  const cases = [...first, ...second].filter(c => selection.test(c.name));
  assert.equal(cases.length, 81);
  for (const c of cases) {
    const expected = await result(reference, c), actual = await result(native, c);
    if (c.name === "y-err-chart-throw") {
      assert.match(actual.message, /^Invalid chart block: expected chart JSON/);
      actual.message = expected.message;
    }
    assert.deepEqual(actual, expected, c.name);
  }
});

test("AbortSignal reason identity survives cancellation", async () => {
  const reason = { code: 123, retry: false };
  for (const api of [reference, native]) {
    const signal = AbortSignal.abort(reason);
    await assert.rejects(api.convertMarkdownToBuffer("x", { signal }), error => {
      assert.deepEqual(Object.keys(error.context), ["reason"]);
      assert.equal(error.context.reason, reason);
      return true;
    });
  }
});

test("throwing asynchronous chart callbacks preserve the contract", async () => {
  const markdown = '```chart\n{"type":"bar","data":{"datasets":[{"data":[1]}]}}\n```';
  const c = { markdown, options: () => ({ chartRendering: { enabled: true, invalidDefinitionBehavior: "throw", renderer: async () => { throw new Error("renderer failed"); } } }) };
  assert.deepEqual(await result(native, c), await result(reference, c));
});
