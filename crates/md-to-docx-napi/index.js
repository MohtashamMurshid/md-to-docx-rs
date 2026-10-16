import { Blob, Buffer } from "node:buffer";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

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


let compatibilityModule;
function compatibility() {
  compatibilityModule ??= import("@mohtasham/md-to-docx");
  return compatibilityModule;
}

function needsCompatibility(value, path = "options") {
  if (value instanceof RegExp) return true;
  if (typeof value === "function") {
    return !["options.imageHandling.resolve", "options.chartRendering.renderer", "options.mermaidRendering.render"].includes(path);
  }
  if (!value || typeof value !== "object" || ArrayBuffer.isView(value) || value instanceof ArrayBuffer) return false;
  if (path === "options" && ["plugins", "pluginOptions", "onWarning"].some(key => Object.hasOwn(value, key))) return true;
  return Object.keys(value).some(key => needsCompatibility(value[key], `${path}.${key}`));
}

let validationModule;
function validateNativeInput(markdown, options) {
  validationModule ??= import(new URL("dist/validation.js", import.meta.resolve("@mohtasham/md-to-docx/package.json")));
  return validationModule.then(({ validateInput }) => {
    try { validateInput(markdown, options ?? {}); }
    catch (error) { throw normalizeError(error); }
  });
}

function normalizeError(error) {
  if (error?.name === "MarkdownConversionError" && !(error instanceof MarkdownConversionError)) {
    return new MarkdownConversionError(error.message, error.context, { cause: error });
  }
  return error;
}

function chartInput(input, options, signal) {
  const definition = input.definition;
  if (!definition || typeof definition !== "object" || Array.isArray(definition)) throw new Error("chart definition must be a JSON object");
  if (!["bar", "line", "pie", "doughnut"].includes(definition.type)) throw new Error("chart type must be one of bar, line, pie, doughnut");
  if (!Array.isArray(definition.data?.datasets) || definition.data.datasets.length === 0) throw new Error("chart data.datasets must contain at least one dataset");
  definition.data.datasets.forEach((dataset, index) => {
    if (!Array.isArray(dataset?.data)) throw new Error(`chart data.datasets[${index}].data must be an array`);
    if (dataset.data.length === 0 || dataset.data.some(value => typeof value !== "number" || !Number.isFinite(value))) throw new Error(`chart data.datasets[${index}].data must contain finite numbers`);
  });
  if (definition.data.labels !== undefined && (!Array.isArray(definition.data.labels) || definition.data.labels.some(label => typeof label !== "string"))) throw new Error("chart data.labels must be an array of strings");
  const result = { definition, signal };
  for (const axis of ["width", "height"]) {
    const value = definition[axis] ?? input[axis];
    const maximum = options?.[axis === "width" ? "maxWidth" : "maxHeight"] ?? 2000;
    if (!Number.isInteger(value) || value <= 0) throw new Error(`chart ${axis} must be a positive integer`);
    if (value > maximum) throw new Error(`chart ${axis} exceeds maximum ${maximum}`);
    result[axis] = value;
  }
  return result;
}

function imageContentType(bytes) {
  if (bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return "image/png";
  if (bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255) return "image/jpeg";
  if (/^GIF8[79]a$/.test(bytes.subarray(0, 6).toString("ascii"))) return "image/gif";
  return undefined;
}

async function invoke(name, markdown, reference, patches, options, type) {
  if (needsCompatibility(options) || needsCompatibility(patches, "patches")
      || typeof reference === "string" || reference instanceof Promise || Array.isArray(reference)
      || (patches && Object.values(patches).some(patch => patch && typeof patch === "object" && Object.hasOwn(patch, "style")))) {
    const api = await compatibility();
    const args = patches !== undefined ? [reference, patches, options]
      : reference !== undefined ? [markdown, reference, options] : [markdown, options];
    // The upstream result already has the public type. Normalize only errors.
    try {
      return await api[name](...args);
    } catch (error) {
      if (error?.name === "MarkdownConversionError") {
        throw new MarkdownConversionError(error.message, error.context, { cause: error });
      }
      throw error;
    }
  }
  if (options?.signal?.aborted) throw abortedError(options.signal);
  if (patches === undefined) {
    if (options !== undefined || typeof markdown !== "string") await validateNativeInput(markdown, options);
  } else if (patches && typeof patches === "object") {
    for (const patch of Object.values(patches)) {
      const source = typeof patch === "string" ? patch : patch?.markdown;
      if (typeof source === "string") await validateNativeInput(source, { ...options, sections: undefined, template: undefined });
    }
  }
  const callbacks = {
    image: typeof options?.imageHandling?.resolve === "function",
    chart: typeof options?.chartRendering?.renderer === "function",
    mermaid: typeof options?.mermaidRendering?.render === "function",
    // Highlighting runs lowlight in JavaScript so token boundaries and theme
    // colors match the TypeScript renderer exactly.
    highlight: options?.codeHighlighting?.enabled === true,
  };
  const referenceBytes = reference === undefined ? undefined : await toBytes(reference);
  if (Object.values(callbacks).some(Boolean)) {
    const plain = { ...options };
    for (const [group, key] of [["imageHandling", "resolve"], ["chartRendering", "renderer"], ["mermaidRendering", "render"]]) {
      if (plain[group]) {
        plain[group] = { ...plain[group] };
        delete plain[group][key];
      }
    }
    const dispatch = async request => {
      const input = JSON.parse(request);
      const signal = options?.signal;
      if (signal?.aborted) throw abortedError(signal);
      let result;
      if (input.kind === "image") result = await options.imageHandling.resolve(input.source, { maxBytes: input.maxBytes, signal });
      if (input.kind === "chart") result = await options.chartRendering.renderer(chartInput(input, options.chartRendering, signal));
      if (input.kind === "mermaid") result = await options.mermaidRendering.render({ code: input.code, meta: input.meta ?? undefined, signal });
      if (input.kind === "highlight") {
        const spans = await highlightSpans(input.code, input.language, options.codeHighlighting);
        return spans ? JSON.stringify(spans) : "null";
      }
      if (result == null) return "null";
      if (typeof result === "string") {
        if (input.kind !== "chart" || !/^data:image\/png;base64,/i.test(result)) throw new Error("chart renderer must return a PNG data URL or PNG bytes");
        return JSON.stringify({ source: result });
      }
      if (input.kind === "chart") return JSON.stringify({ data: Array.from(await toBytes(result)) });
      const bytes = await toBytes(result.data);
      return JSON.stringify({ ...result, contentType: result.contentType ?? imageContentType(bytes), data: Array.from(bytes) });
    };
    return output(native().convertWithCallbacks(markdown ?? "", referenceBytes, patches === undefined ? undefined : JSON.stringify(patches), encodeOptions(plain), JSON.stringify(callbacks), dispatch), type, options);
  }
  const args = patches !== undefined ? [referenceBytes, JSON.stringify(patches), encodeOptions(options)]
    : referenceBytes !== undefined ? [markdown, referenceBytes, encodeOptions(options)] : [markdown, encodeOptions(options)];
  return output(native()[name](...args), type, options);
}

const DOCX_MIME = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

let codeHighlightModule;
/** The compatibility package's lowlight wrapper, loaded from its install location. */
function codeHighlight() {
  codeHighlightModule ??= import(new URL("dist/utils/codeHighlight.js", import.meta.resolve("@mohtasham/md-to-docx/package.json")));
  return codeHighlightModule;
}

/** `tokenizeToRuns` without the docx runs: `[{text, color}]`, or null for an unknown language. */
async function highlightSpans(code, language, codeHighlighting) {
  try {
    const { getLowlightInstance, resolveTheme, DEFAULT_CODE_HIGHLIGHT_THEME } = await codeHighlight();
    const lowlight = getLowlightInstance(codeHighlighting?.languages);
    if (!lowlight.registered(language)) return null;
    const tree = lowlight.highlight(language, code);
    const theme = resolveTheme(codeHighlighting?.theme);
    const spans = [];
    const colorFor = stack => {
      for (let i = stack.length - 1; i >= 0; i--) {
        const normalized = stack[i].startsWith("hljs-") ? stack[i].slice(5) : stack[i];
        if (typeof theme[normalized] === "string") return theme[normalized];
        const dot = normalized.indexOf(".");
        if (dot > 0 && typeof theme[normalized.slice(0, dot)] === "string") return theme[normalized.slice(0, dot)];
      }
      return undefined;
    };
    const walk = (nodes, stack) => {
      for (const node of nodes) {
        if (node.type === "text") {
          if (node.value.length > 0) spans.push({ text: node.value, color: colorFor(stack) || theme.default || DEFAULT_CODE_HIGHLIGHT_THEME.default || "24292E" });
        } else if (node.type === "element") {
          const className = node.properties?.className;
          const classes = Array.isArray(className) ? className.filter(c => typeof c === "string") : typeof className === "string" ? className.split(/\s+/).filter(Boolean) : [];
          walk(node.children, [...stack, ...classes]);
        }
      }
    };
    walk(tree.children, []);
    return spans;
  } catch {
    return null;
  }
}

export class MarkdownConversionError extends Error {
  constructor(message, context, options) {
    super(message, options);
    this.name = "MarkdownConversionError";
    this.context = context;
  }
}

function abortedError(signal) {
  return new MarkdownConversionError("Markdown conversion was aborted", { reason: signal?.reason });
}

async function output(promise, type, options) {
  let bytes;
  let onAbort;
  const signal = options?.signal;
  try {
    if (signal && typeof signal.addEventListener === "function") {
      bytes = await Promise.race([
        promise,
        new Promise((_, reject) => {
          onAbort = () => reject(abortedError(signal));
          signal.addEventListener("abort", onAbort, { once: true });
          if (signal.aborted) onAbort();
        }),
      ]);
    } else {
      bytes = await promise;
    }
  } catch (error) {
    const prefix = "MD_TO_DOCX_ERROR:";
    if (typeof error?.message === "string" && error.message.startsWith(prefix)) {
      const details = JSON.parse(error.message.slice(prefix.length));
      const context = details.context ?? (details.message.startsWith("Failed to convert markdown to docx:") || details.message.startsWith("Invalid chart block:") ? { originalError: {} } : undefined);
      if (context?.originalError && Object.keys(context.originalError).length === 0) {
        context.originalError = new Error(details.message.replace(/^Failed to convert markdown to docx: /, ""));
      }
      throw new MarkdownConversionError(details.message, context, { cause: error });
    }
    if (error?.name === "MarkdownConversionError" && !(error instanceof MarkdownConversionError)) {
      throw new MarkdownConversionError(error.message, error.context, { cause: error });
    }
    throw error;
  } finally {
    if (onAbort) signal.removeEventListener("abort", onAbort);
  }
  if (type === "blob") return new Blob([bytes], { type: DOCX_MIME });
  if (type === "arrayBuffer") {
    return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  }
  return bytes;
}

function encodeOptions(options) {
  if (options === undefined) {
    return undefined;
  }
  if (options?.signal?.aborted) throw abortedError(options.signal);
  rejectFunctions(options, "options");
  return JSON.stringify(options.signal ? { ...options, signal: { aborted: options.signal.aborted === true } } : options);
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

/** docx object trees remain owned by the TypeScript compatibility implementation. */
export async function parseToDocxOptions(markdown, options) {
  try {
    return await (await compatibility()).parseToDocxOptions(markdown, options);
  } catch (error) {
    if (error?.name === "MarkdownConversionError") {
      throw new MarkdownConversionError(error.message, error.context, { cause: error });
    }
    throw error;
  }
}

export async function convertMarkdownToDocx(markdown, options) {
  return invoke("convertMarkdownToDocx", markdown, undefined, undefined, options, "blob");
}

export async function convertMarkdownToArrayBuffer(markdown, options) {
  return invoke("convertMarkdownToArrayBuffer", markdown, undefined, undefined, options, "arrayBuffer");
}

export async function convertMarkdownToBuffer(markdown, options) {
  return invoke("convertMarkdownToBuffer", markdown, undefined, undefined, options, "buffer");
}

export async function convertMarkdownWithReferenceDocx(markdown, referenceDocx, options) {
  return invoke("convertMarkdownWithReferenceDocx", markdown, referenceDocx, undefined, options, "blob");
}

export async function convertMarkdownWithReferenceDocxToArrayBuffer(markdown, referenceDocx, options) {
  return invoke("convertMarkdownWithReferenceDocxToArrayBuffer", markdown, referenceDocx, undefined, options, "arrayBuffer");
}

export async function convertMarkdownWithReferenceDocxToBuffer(markdown, referenceDocx, options) {
  return invoke("convertMarkdownWithReferenceDocxToBuffer", markdown, referenceDocx, undefined, options, "buffer");
}

export async function patchMarkdownInDocx(referenceDocx, patches, options) {
  return invoke("patchMarkdownInDocx", undefined, referenceDocx, patches, options, "blob");
}

export async function patchMarkdownInDocxToArrayBuffer(referenceDocx, patches, options) {
  return invoke("patchMarkdownInDocxToArrayBuffer", undefined, referenceDocx, patches, options, "arrayBuffer");
}

export async function patchMarkdownInDocxToBuffer(referenceDocx, patches, options) {
  return invoke("patchMarkdownInDocxToBuffer", undefined, referenceDocx, patches, options, "buffer");
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
