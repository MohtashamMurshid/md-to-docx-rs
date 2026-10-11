import { Buffer } from "node:buffer";
import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));

function loadNative() {
  const names = [
    "md-to-docx-napi.node",
    join("..", "..", "target", "debug", "libmd_to_docx_napi.so"),
    join("..", "..", "target", "release", "libmd_to_docx_napi.so"),
    join("..", "..", "target", "debug", "deps", "libmd_to_docx_napi.so"),
    join("..", "..", "target", "release", "deps", "libmd_to_docx_napi.so"),
    join("..", "..", "target", "debug", "libmd_to_docx_napi.dylib"),
    join("..", "..", "target", "release", "libmd_to_docx_napi.dylib"),
    join("..", "..", "target", "debug", "md_to_docx_napi.dll"),
    join("..", "..", "target", "release", "md_to_docx_napi.dll"),
  ];
  const failures = [];
  for (const name of names) {
    const candidate = name.startsWith("..") ? join(here, name) : join(here, name);
    if (!existsSync(candidate)) {
      continue;
    }
    try {
      const loaded = { exports: {} };
      process.dlopen(loaded, candidate);
      return loaded.exports;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      failures.push(`${candidate}: ${message}`);
    }
  }
  const detail = failures.length > 0 ? failures.join("; ") : "no build output was found";
  throw new Error(`Cannot load the md-to-docx native binding. ${detail}`);
}

let nativeBinding;

function native() {
  if (!nativeBinding) {
    nativeBinding = loadNative();
  }
  return nativeBinding;
}

function encodeOptions(options) {
  if (options === undefined) {
    return undefined;
  }
  rejectFunctions(options, "options");
  return JSON.stringify(options);
}

function rejectFunctions(value, path) {
  if (typeof value === "function") {
    throw new Error(`${path} is a function and cannot be passed from JavaScript`);
  }
  if (!value || typeof value !== "object") {
    return;
  }
  if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer) {
    return;
  }
  for (const key of Object.keys(value)) {
    rejectFunctions(value[key], `${path}.${key}`);
  }
}

async function toBytes(value) {
  if (Buffer.isBuffer(value)) {
    return value;
  }
  if (value instanceof Uint8Array) {
    return Buffer.from(value);
  }
  if (value instanceof ArrayBuffer) {
    return Buffer.from(value);
  }
  if (typeof Blob !== "undefined" && value instanceof Blob) {
    return Buffer.from(await value.arrayBuffer());
  }
  throw new Error("Expected docx bytes");
}

export async function convertMarkdownToDocx(markdown, options) {
  return native().convertMarkdownToDocx(markdown, encodeOptions(options));
}

export async function convertMarkdownToArrayBuffer(markdown, options) {
  return native().convertMarkdownToArrayBuffer(markdown, encodeOptions(options));
}

export async function convertMarkdownToBuffer(markdown, options) {
  return native().convertMarkdownToBuffer(markdown, encodeOptions(options));
}

export async function convertMarkdownWithReferenceDocx(markdown, referenceDocx, options) {
  return native().convertMarkdownWithReferenceDocx(
    markdown,
    await toBytes(referenceDocx),
    encodeOptions(options),
  );
}

export async function convertMarkdownWithReferenceDocxToArrayBuffer(
  markdown,
  referenceDocx,
  options,
) {
  return native().convertMarkdownWithReferenceDocxToArrayBuffer(
    markdown,
    await toBytes(referenceDocx),
    encodeOptions(options),
  );
}

export async function convertMarkdownWithReferenceDocxToBuffer(markdown, referenceDocx, options) {
  return native().convertMarkdownWithReferenceDocxToBuffer(
    markdown,
    await toBytes(referenceDocx),
    encodeOptions(options),
  );
}

export async function patchMarkdownInDocx(referenceDocx, patches, options) {
  rejectFunctions(patches, "patches");
  return native().patchMarkdownInDocx(
    await toBytes(referenceDocx),
    JSON.stringify(patches),
    encodeOptions(options),
  );
}

export async function patchMarkdownInDocxToArrayBuffer(referenceDocx, patches, options) {
  rejectFunctions(patches, "patches");
  return native().patchMarkdownInDocxToArrayBuffer(
    await toBytes(referenceDocx),
    JSON.stringify(patches),
    encodeOptions(options),
  );
}

export async function patchMarkdownInDocxToBuffer(referenceDocx, patches, options) {
  rejectFunctions(patches, "patches");
  return native().patchMarkdownInDocxToBuffer(
    await toBytes(referenceDocx),
    JSON.stringify(patches),
    encodeOptions(options),
  );
}

/**
 * Browser download. Node has no `window`, so this throws a plain Error.
 * @param {Blob} blob
 * @param {string} [filename]
 */
export async function downloadDocx(blob, filename = "document.docx") {
  if (typeof window === "undefined") {
    throw new Error("This function can only be used in browser environments");
  }
  if (!(blob instanceof Blob)) {
    throw new Error("Invalid blob provided");
  }
  if (!filename || typeof filename !== "string") {
    throw new Error("Invalid filename provided");
  }
  throw new Error("This function can only be used in browser environments");
}
