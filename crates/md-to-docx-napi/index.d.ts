import type { Buffer } from "node:buffer";
import type { Options, ReferenceDocxGenerationOptions, PatchMarkdownOptions, MarkdownConversionErrorContext, MarkdownDocxPatch, ReferenceDocxInput } from "@mohtasham/md-to-docx";
export type * from "@mohtasham/md-to-docx";
export class MarkdownConversionError extends Error {
  context?: MarkdownConversionErrorContext;
  constructor(message: string, context?: MarkdownConversionErrorContext, options?: ErrorOptions);
}
export type ReferenceDocxBytes = Blob | ArrayBuffer | Uint8Array | Buffer;
export type Patches = Record<string, MarkdownDocxPatch>;
export function convertMarkdownToDocx(markdown: string, options?: Options): Promise<Blob>;
export function convertMarkdownToArrayBuffer(markdown: string, options?: Options): Promise<ArrayBuffer>;
export function convertMarkdownToBuffer(markdown: string, options?: Options): Promise<Buffer>;
export function convertMarkdownWithReferenceDocx(markdown: string, referenceDocx: ReferenceDocxBytes, options?: ReferenceDocxGenerationOptions): Promise<Blob>;
export function convertMarkdownWithReferenceDocxToArrayBuffer(markdown: string, referenceDocx: ReferenceDocxBytes, options?: ReferenceDocxGenerationOptions): Promise<ArrayBuffer>;
export function convertMarkdownWithReferenceDocxToBuffer(markdown: string, referenceDocx: ReferenceDocxBytes, options?: ReferenceDocxGenerationOptions): Promise<Buffer>;
export function patchMarkdownInDocx(referenceDocx: ReferenceDocxInput, patches: Patches, options?: PatchMarkdownOptions): Promise<Blob>;
export function patchMarkdownInDocxToArrayBuffer(referenceDocx: ReferenceDocxInput, patches: Patches, options?: PatchMarkdownOptions): Promise<ArrayBuffer>;
export function patchMarkdownInDocxToBuffer(referenceDocx: ReferenceDocxInput, patches: Patches, options?: PatchMarkdownOptions): Promise<Buffer>;
export function downloadDocx(blob: Blob, filename?: string): Promise<never>;
export declare const parseToDocxOptions: typeof import("@mohtasham/md-to-docx").parseToDocxOptions;
