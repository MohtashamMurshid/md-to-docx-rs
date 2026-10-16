import assert from "node:assert/strict";
import { test } from "node:test";
import {
  convertMarkdownToDocx, convertMarkdownToArrayBuffer, convertMarkdownToBuffer,
  convertMarkdownWithReferenceDocx, convertMarkdownWithReferenceDocxToArrayBuffer,
  convertMarkdownWithReferenceDocxToBuffer, patchMarkdownInDocx,
  patchMarkdownInDocxToArrayBuffer, patchMarkdownInDocxToBuffer,
  MarkdownConversionError,
} from "../index.js";

const mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
// Separate conversions may cross a ZIP timestamp boundary. Compare every
// unpacked part, including metadata and media, without ignoring content bytes.
async function checkPackageContents(actual, expected) {
  const [a, b] = await Promise.all([parts(actual), parts(expected)]);
  assert.deepEqual(Object.keys(a.files).sort(), Object.keys(b.files).sort());
  for (const name of Object.keys(a.files)) {
    assert.equal(a.files[name].dir, b.files[name].dir, name);
    assert.deepEqual(await a.files[name].async("nodebuffer"), await b.files[name].async("nodebuffer"), name);
  }
}

async function checkOutputs(functions, args) {
  const blob = await functions[0](...args);
  const array = await functions[1](...args);
  const buffer = await functions[2](...args);
  assert.ok(blob instanceof Blob);
  assert.equal(blob.type, mime);
  assert.ok(array instanceof ArrayBuffer);
  assert.ok(Buffer.isBuffer(buffer));
  await checkPackageContents(Buffer.from(await blob.arrayBuffer()), buffer);
  await checkPackageContents(Buffer.from(array), buffer);
  assert.equal(buffer.subarray(0, 2).toString(), "PK");
}

test("conversion returns the advertised output types and identical DOCX parts", async () => {
  await checkOutputs([convertMarkdownToDocx, convertMarkdownToArrayBuffer, convertMarkdownToBuffer], ["# Hello\n\nWorld"]);
});

test("reference conversion accepts Blob, ArrayBuffer, Buffer and Uint8Array", async () => {
  const reference = await convertMarkdownToBuffer("# Template");
  for (const input of [reference, new Uint8Array(reference), reference.buffer.slice(reference.byteOffset, reference.byteOffset + reference.byteLength), new Blob([reference])]) {
    await checkOutputs([convertMarkdownWithReferenceDocx, convertMarkdownWithReferenceDocxToArrayBuffer, convertMarkdownWithReferenceDocxToBuffer], ["# Replacement", input]);
  }
});

test("patch conversion returns the advertised output types", async () => {
  const reference = await convertMarkdownToBuffer("Hello {{body}}");
  await checkOutputs([patchMarkdownInDocx, patchMarkdownInDocxToArrayBuffer, patchMarkdownInDocxToBuffer], [reference, { body: "**Replacement**" }]);
});

test("conversion leaves the event loop available while native work runs", async () => {
  const markdown = "A **bold** paragraph with [a link](https://example.com).\n\n".repeat(12000);
  let finished = false;
  const conversion = convertMarkdownToBuffer(markdown).then(bytes => { finished = true; return bytes; });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(finished, false, "conversion completed before the timer; native work may block the event loop");
  assert.ok((await conversion).length > 0);
});

test("conversion errors retain their name, message and core context", async () => {
  await assert.rejects(convertMarkdownToBuffer("Hello", { style: { fontFamily: " " } }), error => {
    assert.ok(error instanceof MarkdownConversionError);
    assert.match(error.message, /Invalid fontFamily/);
    return true;
  });
  await assert.rejects(patchMarkdownInDocxToBuffer(await convertMarkdownToBuffer("{{missing}}"), { missing: "<!-- COMMENT: reviewer note -->" }), error => {
    assert.ok(error instanceof MarkdownConversionError);
    assert.equal(error.context.placeholder, "missing");
    return true;
  });
});



test("AbortSignal rejects before conversion and while worker conversion runs", async () => {
  await assert.rejects(convertMarkdownToBuffer("Hello", { signal: AbortSignal.abort() }), error => error.context?.reason?.name === "AbortError");
  const controller = new AbortController();
  const conversion = convertMarkdownToBuffer("A paragraph.\n\n".repeat(12000), { signal: controller.signal });
  controller.abort();
  await assert.rejects(conversion, error => error.context?.reason?.name === "AbortError");
});

const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aL1sAAAAASUVORK5CYII=", "base64");
async function parts(bytes) {
  const { default: JSZip } = await import("jszip");
  return JSZip.loadAsync(bytes, { checkCRC32: true });
}
async function documentText(bytes) {
  return (await parts(bytes)).file("word/document.xml").async("string");
}

test("output comparison accepts ZIP timestamp changes and rejects changed document parts", async () => {
  const bytes = await convertMarkdownToBuffer("# Package contents");
  const zip = await parts(bytes);
  for (const entry of Object.values(zip.files)) entry.date = new Date("2001-01-01T00:00:00Z");
  const repacked = await zip.generateAsync({ type: "nodebuffer" });
  assert.notDeepEqual(repacked, bytes);
  await checkPackageContents(repacked, bytes);
  const xml = await zip.file("word/document.xml").async("string");
  zip.file("word/document.xml", xml.replace("Package contents", "Changed contents"), { createFolders: false });
  const changed = await zip.generateAsync({ type: "nodebuffer" });
  await assert.rejects(checkPackageContents(changed, bytes), { message: /word\/document\.xml/ });
});

test("native image resolver awaits async JavaScript and embeds its bytes", async () => {
  const calls = [];
  const bytes = await convertMarkdownToBuffer("![diagram](virtual:diagram)", {
    imageHandling: { resolve: async (source, context) => {
      calls.push(source);
      assert.equal(context.maxBytes, 5 * 1024 * 1024);
      await new Promise(resolve => setTimeout(resolve, 5));
      return { data: png, contentType: "image/png" };
    } },
  });
  assert.deepEqual(calls, ["virtual:diagram"]);
  assert.deepEqual(await (await parts(bytes)).file("word/media/image1.png").async("nodebuffer"), png);
});

test("native chart renderer accepts bytes and data URLs", async () => {
  for (const result of [png, `data:image/png;base64,${png.toString("base64")}`]) {
    const bytes = await convertMarkdownToBuffer('```chart\n{"type":"bar","data":{"labels":["A"],"datasets":[{"data":[1]}]}}\n```', {
      chartRendering: { enabled: true, renderer: async input => {
        assert.equal(input.definition.type, "bar");
        assert.equal(input.width, 640);
        assert.equal(input.height, 360);
        return result;
      } },
    });
    assert.ok((await parts(bytes)).file("word/media/image1.png"));
  }
});

test("native Mermaid renderer receives code and embeds output", async () => {
  const bytes = await convertMarkdownToBuffer("```mermaid title\ngraph LR; A-->B\n```", {
    mermaidRendering: { enabled: true, render: async input => {
      assert.match(input.code, /graph LR/);
      assert.equal(input.meta, "title");
      return { data: png, contentType: "image/png", altText: "Flow chart", title: "Diagram" };
    } },
  });
  assert.match(await documentText(bytes), /Flow chart/);
  assert.ok((await parts(bytes)).file("word/media/image1.png"));
});

test("concurrent callback conversions can await filesystem operations", { timeout: 20000 }, async () => {
  const { readFile } = await import("node:fs/promises");
  await Promise.all(Array.from({ length: 8 }, () => convertMarkdownToBuffer("![diagram](virtual:diagram)", {
    imageHandling: { resolve: async () => {
      await readFile(new URL("../package.json", import.meta.url));
      return { data: png, contentType: "image/png" };
    } },
  })));
});

test("renderer failures preserve diagnostics and honor throw behavior", async () => {
  await assert.rejects(convertMarkdownToBuffer('```chart\n{"type":"bar","data":{"datasets":[{"data":[1]}]}}\n```', {
    chartRendering: { enabled: true, invalidDefinitionBehavior: "throw", renderer: async () => { throw new Error("renderer failed"); } },
  }), /renderer failed/);
});

test("plugin compatibility runs async setup, mutable mdast transforms and fence handlers", async () => {
  const calls = [];
  const plugin = {
    apiVersion: 1, name: "test.compatibility",
    async setup() { calls.push("setup"); return { prefix: "Plugin" }; },
    async transformAst(root) {
      calls.push("transform");
      root.children.unshift({ type: "paragraph", children: [{ type: "text", value: "AST inserted" }] });
    },
    fencedBlocks: [{ languages: ["custom"], async render(input, context) {
      calls.push("fence");
      return { type: "paragraph", children: `${context.state.prefix}: ${input.value}` };
    } }],
  };
  const bytes = await convertMarkdownToBuffer("```custom\ncontent\n```", { plugins: [plugin] });
  const xml = await documentText(bytes);
  assert.match(xml, /AST inserted/);
  assert.match(xml, /Plugin: content/);
  assert.deepEqual(calls, ["setup", "transform", "fence"]);
});

test("RegExp and function text replacements retain original AST semantics", async () => {
  const bytes = await convertMarkdownToBuffer("Hello world\n\n`world`", {
    textReplacements: [{ find: /world/g, replace: match => match.toUpperCase() }],
  });
  const xml = await documentText(bytes);
  assert.match(xml, /WORLD/);
  assert.match(xml, /world/); // Inline code is excluded by mdast find-and-replace.
});

test("native callbacks work in reference generation and placeholder patches", async () => {
  const template = await convertMarkdownToBuffer("{{body}}");
  let calls = 0;
  const options = { imageHandling: { resolve: async () => { calls++; return { data: png }; } } };
  for (const fn of [convertMarkdownWithReferenceDocx, convertMarkdownWithReferenceDocxToArrayBuffer, convertMarkdownWithReferenceDocxToBuffer]) {
    const value = await fn("![resolved](virtual:image)", template, options);
    const bytes = value instanceof Blob ? await value.arrayBuffer() : value;
    assert.ok((await parts(bytes)).file("word/media/image1.png"));
  }
  for (const fn of [patchMarkdownInDocx, patchMarkdownInDocxToArrayBuffer, patchMarkdownInDocxToBuffer]) {
    const value = await fn(template, { body: "![resolved](virtual:image)" }, options);
    const bytes = value instanceof Blob ? await value.arrayBuffer() : value;
    const images = (await parts(bytes)).file(/^word\/media\/.*\.png$/);
    assert.equal(images.length, 1);
    assert.deepEqual(await images[0].async("nodebuffer"), png);
  }
  assert.equal(calls, 6);
});

test("plugin fallback works for reference and patch output families", async () => {
  const template = await convertMarkdownToBuffer("{{body}}");
  const options = { plugins: [{ apiVersion: 1, name: "test.plugin", fencedBlocks: [{ languages: ["custom"], render() { return { type: "paragraph", children: "Plugin output" }; } }] }] };
  for (const fn of [convertMarkdownWithReferenceDocx, convertMarkdownWithReferenceDocxToArrayBuffer, convertMarkdownWithReferenceDocxToBuffer]) {
    const value = await fn("```custom\ninput\n```", template, options);
    const bytes = value instanceof Blob ? await value.arrayBuffer() : value;
    assert.match(await documentText(bytes), /Plugin output/);
  }
  for (const fn of [patchMarkdownInDocx, patchMarkdownInDocxToArrayBuffer, patchMarkdownInDocxToBuffer]) {
    const value = await fn(template, { body: "```custom\ninput\n```" }, options);
    const bytes = value instanceof Blob ? await value.arrayBuffer() : value;
    assert.match(await documentText(bytes), /Plugin output/);
  }
});

test("chart callback receives definition dimensions and malformed schema does not reach it", async () => {
  let calls = 0;
  const options = { chartRendering: { enabled: true, invalidDefinitionBehavior: "throw", renderer: input => {
    calls++;
    assert.equal(input.width, 320);
    assert.equal(input.height, 180);
    return png;
  } } };
  const bytes = await convertMarkdownToBuffer('```chart\n{"type":"bar","width":320,"height":180,"data":{"datasets":[{"data":[1]}]}}\n```', options);
  const xml = await documentText(bytes);
  assert.match(xml, /cx="3048000"/); // 320 px * 9525 EMU.
  assert.match(xml, /cy="1714500"/);
  await assert.rejects(convertMarkdownToBuffer('```chart\n{"type":"bar","data":{"datasets":[]}}\n```', options), /at least one dataset/);
  assert.equal(calls, 1);
});

test("Mermaid callback detects JPEG format without a contentType hint", async () => {
  const { default: sharp } = await import("sharp");
  const jpeg = await sharp({ create: { width: 2, height: 2, channels: 3, background: "white" } }).jpeg().toBuffer();
  const bytes = await convertMarkdownToBuffer("```mermaid\ngraph LR; A-->B\n```", {
    mermaidRendering: { enabled: true, render: () => ({ data: jpeg, altText: "Diagram" }) },
  });
  const zip = await parts(bytes);
  const images = zip.file(/^word\/media\/.*\.(jpg|jpeg)$/);
  assert.equal(images.length, 1);
  assert.deepEqual(await images[0].async("nodebuffer"), jpeg);
});

test("warning callbacks run through the TypeScript compatibility implementation", async () => {
  const warnings = [];
  await convertMarkdownToBuffer("![missing](https://example.com/missing.png)", { onWarning: warning => warnings.push(warning) });
  assert.ok(warnings.length > 0);
});

test("native highlighting matches lowlight token boundaries, colors and NBSP spacing", async () => {
  const upstream = await import("@mohtasham/md-to-docx");
  const markdown = '```ts\nconst n: number = 1;\n  // comment\n```\n\n```unknown-language\n  plain\n```';
  const options = { codeHighlighting: { enabled: true, theme: { keyword: "123456", comment: "654321" } } };
  const tokens = xml => [...xml.matchAll(/<w:r\b[^>]*>([\s\S]*?)<\/w:r>/g)].map(([, run]) => ({
    text: [...run.matchAll(/<w:t\b[^>]*>([\s\S]*?)<\/w:t>/g)].map(match => match[1]).join(""),
    color: /<w:color\b[^>]*w:val="([^"]+)"/.exec(run)?.[1],
    breaks: [...run.matchAll(/<w:br\b/g)].length,
  }));
  assert.deepEqual(tokens(await documentText(await convertMarkdownToBuffer(markdown, options))),
    tokens(await documentText(await upstream.convertMarkdownToBuffer(markdown, options))));
});

test("invalid callback options reject without aborting the Node process", async () => {
  await assert.rejects(convertMarkdownToBuffer("```js\nx\n```", {
    codeHighlighting: { enabled: true }, toc: { minDepth: 0 },
  }), /Invalid TOC minDepth/);
  assert.ok((await convertMarkdownToBuffer("Still working")).length > 0);
});
