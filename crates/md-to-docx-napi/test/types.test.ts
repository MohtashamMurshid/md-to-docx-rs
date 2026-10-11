import {
  convertMarkdownToDocx, convertMarkdownToArrayBuffer, convertMarkdownToBuffer,
  convertMarkdownWithReferenceDocx, patchMarkdownInDocxToBuffer, parseToDocxOptions,
  type Options, type MarkdownDocxPlugin,
} from "../index.js";
import type { Buffer } from "node:buffer";

const plugin: MarkdownDocxPlugin<{ prefix: string }> = {
  apiVersion: 1,
  name: "consumer.example",
  async setup() { return { prefix: "Example" }; },
  transformAst(root, context) {
    root.children.push({ type: "paragraph", children: [{ type: "text", value: context.state.prefix }] });
  },
  fencedBlocks: [{ languages: ["example"], render(input, context) {
    return { type: "paragraph", children: `${context.state.prefix} ${input.value}` };
  } }],
};
const options: Options = {
  plugins: [plugin],
  codeHighlighting: { enabled: true },
  imageHandling: { resolve: async (_source, context) => {
    const signal: AbortSignal | undefined = context.signal;
    void signal;
    return { data: new Uint8Array(context.maxBytes > 0 ? 1 : 0), contentType: "image/png" };
  } },
};
const blob: Promise<Blob> = convertMarkdownToDocx("Hello", options);
const array: Promise<ArrayBuffer> = convertMarkdownToArrayBuffer("Hello", options);
const buffer: Promise<Buffer> = convertMarkdownToBuffer("Hello", options);
const reference: Promise<Blob> = convertMarkdownWithReferenceDocx("Hello", new Uint8Array(), options);
const patched: Promise<Buffer> = patchMarkdownInDocxToBuffer(new Uint8Array(), { body: { markdown: "Hello", style: { fontFamily: "Arial" } } });
const objects = parseToDocxOptions("Hello", options);
void [blob, array, buffer, reference, patched, objects];
// @ts-expect-error ArrayBuffer functions do not return Blob.
const wrong: Promise<Blob> = convertMarkdownToArrayBuffer("Hello");
void wrong;
