// Second hand-written parity set: options, error messages, rich tables,
// reference and patch paths, plus seeded random documents.
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const png1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
const referencePath = join(here, "..", "..", "..", "crates", "md-to-docx", "tests", "oracle", "fixtures", "reference-template.docx");
const reference = () => new Uint8Array(readFileSync(referencePath));

const cases = [];
const add = (name, markdown, options, extra) => cases.push({ name: `y-${name}`, markdown, options, ...extra });
add("math-in-ordered-list", "1. First\n\n   $$\n   x^2\n   $$\n\n2. Second", {});
add("math-in-unordered-list", "- First\n\n  $$\n  x^2\n  $$\n\n- Second", {});

// ---- option validation errors (message parity)
const bad = (name, options, markdown = "# T\n\nbody") => add(`err-${name}`, markdown, options);
bad("title-size-low", { style: { titleSize: 4 } });
bad("title-size-high", { style: { titleSize: 100 } });
bad("heading-spacing", { style: { headingSpacing: 800 } });
bad("paragraph-spacing", { style: { paragraphSpacing: 721 } });
bad("line-spacing-low", { style: { lineSpacing: 0.5 } });
bad("line-spacing-high", { style: { lineSpacing: 3.5 } });
bad("font-empty", { style: { fontFamily: "  " } });
bad("language-bad", { style: { language: "not a tag" } });
bad("table-widths-empty", { style: { tableColumnWidths: [] } });
bad("table-widths-frac", { style: { tableColumnWidths: [100.5, 200] } });
bad("table-widths-big", { style: { tableColumnWidths: [40000] } });
bad("table-row-split", { style: { tableAllowRowSplit: "yes" } });
bad("table-header-bg", { style: { tableHeaderBackground: "red" } });
bad("table-margins", { style: { tableCellMargins: { top: -1 } } });
bad("inline-code-color", { style: { inlineCodeColor: "#FFFFFF" } });
bad("inline-code-bg", { style: { inlineCodeBackground: 123456 } });
bad("callout-not-object", { style: { calloutStyles: "x" } });
bad("callout-key", { style: { calloutStyles: { danger: {} } } });
bad("callout-value", { style: { calloutStyles: { note: "blue" } } });
bad("callout-hex", { style: { calloutStyles: { note: { borderColor: "zzz" } } } });
bad("toc-mode", { toc: { enabled: true, mode: "weird" } });
bad("toc-title", { toc: { title: 5 } });
bad("toc-depth", { toc: { minDepth: 0 } });
bad("toc-depth-range", { toc: { minDepth: 3, maxDepth: 2 } });
bad("math-enabled", { mathRendering: { enabled: "yes" } });
bad("math-unsupported", { mathRendering: { unsupported: "ignore" } });
bad("captions-label", { captions: { figureLabel: " " } });
bad("captions-placement", { captions: { figurePlacement: "middle" } });
bad("captions-alignment", { captions: { alignment: "MIDDLE" } });
bad("captions-size", { captions: { size: 4 } });
bad("captions-failure", { captions: { failureMode: "explode" } });
bad("max-input", { maxInputLength: 0 });
bad("max-elements", { maxElements: -1 });
bad("chart-width", { chartRendering: { width: 0 } });
bad("chart-invalid-behavior", { chartRendering: { invalidDefinitionBehavior: "ignore" } });
bad("mermaid-failure", { mermaidRendering: { failureMode: "nope" } });
bad("image-max", { imageHandling: { maxImages: 0 } });
bad("image-hosts", { imageHandling: { remote: { allowedHosts: [""] } } });
bad("section-type", { sections: [{ markdown: "x", type: "WEIRD" }] });
bad("section-empty", { sections: [] });
bad("section-markdown", { sections: [{}] });
bad("page-margin", { sections: [{ markdown: "x", page: { margin: { top: -5 } } }] });
bad("page-size", { sections: [{ markdown: "x", page: { size: { width: 0 } } }] });
bad("page-orientation", { sections: [{ markdown: "x", page: { size: { orientation: "SIDEWAYS" } } }] });
bad("page-numbering-start", { sections: [{ markdown: "x", pageNumbering: { start: 0 } }] });
bad("page-numbering-display", { sections: [{ markdown: "x", pageNumbering: { display: "all" } }] });
bad("page-numbering-format", { sections: [{ markdown: "x", pageNumbering: { formatType: "roman" } }] });
bad("header-slot", { sections: [{ markdown: "x", headers: { default: 5 } }] });
bad("header-alignment", { sections: [{ markdown: "x", headers: { default: { text: "h", alignment: "MIDDLE" } } }] });
bad("title-page", { sections: [{ markdown: "x", titlePage: "yes" } ] });
bad("accessibility", { accessibility: { missingImageAltText: "warn" } });
bad("metadata-title", { metadata: { title: 5 } });
bad("metadata-language", { metadata: { language: "bad tag!" } });
bad("metadata-created", { metadata: { created: "not a date" } });
bad("text-replacement-mode", { textReplacements: [{ find: "a", replace: "b" }], textReplacementMode: "sometimes" });
bad("missing-alt-throw", { accessibility: { missingImageAltText: "throw" } }, "![](" + png1x1 + ")");
bad("math-throw", { mathRendering: { unsupported: "throw" } }, "$\\unknown{x}$");
bad("header-footnote", { sections: [{ markdown: "x", headers: { default: { markdown: "a[^1]\n\n[^1]: b" } } }] });
bad("max-input-length-hit", { maxInputLength: 3 });
bad("max-elements-hit", { maxElements: 1 });
bad("chart-throw", { chartRendering: { enabled: true, invalidDefinitionBehavior: "throw" } }, "```chart\nnope\n```");
bad("mermaid-throw", { mermaidRendering: { enabled: true, failureMode: "throw" } }, "```mermaid\ngraph\n```");
bad("rich-table-json", {}, "```table\n{bad\n```");
bad("rich-table-rows", {}, "```table\n{\"rows\":[]}\n```");
bad("rich-table-span", {}, "```table\n{\"rows\":[[{\"markdown\":\"a\",\"columnSpan\":0}]]}\n```");
bad("rich-table-widths", {}, "```table\n{\"rows\":[[\"a\"]],\"columnWidths\":[0]}\n```");
bad("rich-table-overlap", {}, "```table\n{\"rows\":[[{\"markdown\":\"a\",\"rowSpan\":2},\"b\"],[\"c\",\"d\"]]}\n```");

// ---- sections, headers, footers, page numbering
add("sections-numbering", "x", { sections: [
  { markdown: "# A", pageNumbering: { display: "currentAndTotal", alignment: "RIGHT", formatType: "upperRoman", start: 3, separator: "hyphen" } },
  { markdown: "# B", pageNumbering: { display: "none" } },
  { markdown: "# C", pageNumbering: { display: "currentAndSectionTotal", alignment: "LEFT", formatType: "lowerLetter", separator: "emDash" } },
] });
add("sections-headers-text", "x", { sections: [
  { markdown: "body", headers: { default: { text: "Head", alignment: "RIGHT", pageNumberDisplay: "current" }, even: { text: "Even" }, first: null }, footers: { default: { text: "Foot", pageNumberDisplay: "none" }, first: { text: "First foot", alignment: "LEFT" } } },
] });
add("sections-header-markdown-only", "x", { sections: [ { markdown: "body", headers: { default: { markdown: "**rich** header" } }, footers: { default: { markdown: "- footer list" , pageNumberDisplay: "currentAndTotal" } } } ] });
add("sections-header-markdown-and-text", "x", { sections: [ { markdown: "body", headers: { default: { markdown: "rich", text: "plain", alignment: "CENTER" } } } ] });
add("sections-page-sizes", "x", { sections: [
  { markdown: "a", page: { size: { width: 12240, height: 15840 }, margin: { top: 720, bottom: 720, left: 1000, right: 1000, header: 300, footer: 300, gutter: 50 } } },
  { markdown: "b", page: { size: { orientation: "LANDSCAPE" } }, type: "CONTINUOUS" },
  { markdown: "c", type: "EVEN_PAGE", titlePage: false },
  { markdown: "d", type: "ODD_PAGE" },
  { markdown: "e", type: "NEXT_COLUMN" },
] });
add("sections-style-per-section", "x", { style: { paragraphSize: 20 }, template: { style: { titleSize: 40 } }, sections: [ { markdown: "# A\n\npara", style: { paragraphSize: 30, direction: "RTL" } }, { markdown: "# B\n\npara" } ] });
add("sections-with-toc", "x", { toc: { enabled: true, mode: "links" }, sections: [ { markdown: "[TOC]\n\n# A" }, { markdown: "# B\n\n## C" } ] });
add("sections-footnotes-across", "x", { sections: [ { markdown: "a[^1]\n\n[^1]: one" }, { markdown: "b[^1] c[^2]\n\n[^1]: uno\n[^2]: two" } ] });
add("sections-lists-across", "x", { sections: [ { markdown: "1. a\n2. b" }, { markdown: "1. c\n\n- d" } ] });
add("sections-images-across", "x", { sections: [ { markdown: "![a](" + png1x1 + ")" }, { markdown: "![b](" + png1x1 + ") ![c](" + png1x1 + ")" } ] });
add("sections-empty-markdown", "x", { sections: [ { markdown: "" }, { markdown: "   " }, { markdown: "# ok" } ] });
add("template-only", "# T\n\npara", { template: { headers: { default: { text: "H" } }, page: { margin: { top: 100 } }, pageNumbering: { display: "none" } } });

// ---- TOC variants
add("toc-native-full", "[TOC]\n\n# One\n\n## Two\n\n### Three\n\n#### Four\n\n##### Five\n\n###### Six", { toc: { enabled: true, minDepth: 2, maxDepth: 5, title: "", dotLeaders: false, pageNumbers: false, updateFieldsOnOpen: false } });
add("toc-links-mode", "[TOC]\n\n# One\n\n## Two", { toc: { enabled: true, mode: "links" } });
add("toc-twice", "[TOC]\n\n# One\n\n[TOC]\n\n## Two", { toc: { enabled: true } });
add("toc-styles", "[TOC]\n\n# One\n\n## Two\n\n### Three", { toc: { enabled: true }, style: { tocFontSize: 18, tocHeading1FontSize: 30, tocHeading2Bold: true, tocHeading3Italic: true, tocHeading1Bold: false } });
add("toc-disabled-with-placeholder", "[TOC]\n\n# One", { toc: { enabled: false } });
add("toc-in-list", "- [TOC]\n\n# One", { toc: { enabled: true } });
add("toc-heading-links", "# Intro\n\nSee [intro](#intro) and [missing](#nope) and [dup](#same-1).\n\n# Same\n\n# Same\n\n## Ünïcode Héading!\n\n[u](#ünïcode-héading)", { toc: { enabled: true } });

// ---- captions & cross references
add("captions-full", "![A](" + png1x1 + ")\n\nFigure: First {#fig:a}\n\n| x |\n|---|\n| y |\n\nTable: Tab {#tbl:a}\n\nText [@fig:a], [@tbl:a], again [@fig:a].\n\n![B](" + png1x1 + ")\n\nFigure: Second {#fig:b}\n\n[@fig:b]", { captions: { updateFieldsOnOpen: true } });
add("captions-duplicate-id", "![A](" + png1x1 + ")\n\nFigure: one {#fig:a}\n\n![B](" + png1x1 + ")\n\nFigure: two {#fig:a}\n\n[@fig:a]");
add("captions-duplicate-throw", "![A](" + png1x1 + ")\n\nFigure: one {#fig:a}\n\n![B](" + png1x1 + ")\n\nFigure: two {#fig:a}", { captions: { failureMode: "throw" } });
add("captions-orphan-throw", "Figure: nothing above {#fig:x}", { captions: { failureMode: "throw" } });
add("captions-in-quote-and-list", "> ![A](" + png1x1 + ")\n>\n> Figure: in quote {#fig:q}\n\n- ![B](" + png1x1 + ")\n\n  Figure: in list {#fig:l}\n\n[@fig:q] [@fig:l]");
add("captions-formatting", "![A](" + png1x1 + ")\n\nFigure: **bold** and `code` and [link](https://x.y) {#fig:a}");
add("captions-table-before", "Table: above {#tbl:a}\n\n| x |\n|---|\n| y |", { captions: { tablePlacement: "above" } });

// ---- text replacements (string find only: native path)
add("replacements-basic", "Hello NAME. NAME! name", { textReplacements: [{ find: "NAME", replace: "World" }] });
add("replacements-markdown-result", "Hello NAME", { textReplacements: [{ find: "NAME", replace: "**bold** _em_" }] });
add("replacements-many", "a b c d", { textReplacements: [{ find: "a", replace: "1" }, { find: "b", replace: "2" }, { find: "1", replace: "x" }] });
add("replacements-in-code", "`NAME` and NAME and [NAME](https://x.y/NAME)", { textReplacements: [{ find: "NAME", replace: "W" }] });
add("replacements-too-many", "x", { textReplacements: Array.from({ length: 51 }, (_, i) => ({ find: `k${i}`, replace: "v" })) });
add("replacements-long-pattern", "x", { textReplacements: [{ find: "a".repeat(300), replace: "v" }] });
add("replacements-long-text", "x", { textReplacements: [{ find: "x", replace: "v".repeat(5000) }] });
add("replacements-empty-find", "x", { textReplacements: [{ find: "", replace: "v" }] });

// ---- metadata
add("metadata-full-core", "x", { metadata: { title: "T", subject: "S", description: "D", author: "A", keywords: "k1, k2", category: "C", company: "Co", language: "de", lastModifiedBy: "M", revision: 7, created: "2020-01-02T03:04:05Z", modified: "2021-02-03T04:05:06.789Z", custom: { Project: "X", Count: 3, Flag: true } } });
add("metadata-language-only", "# H\n\npara `code`\n\n> q\n\n- l\n\n| a |\n|---|\n| b |", { metadata: { language: "ja-JP" } });
add("metadata-rtl-language", "# H\n\npara", { metadata: { language: "ar" }, style: { direction: "RTL" } });
add("metadata-custom-only", "x", { metadata: { custom: { A: "1" } } });
add("metadata-bad-custom", "x", { metadata: { custom: { "": "1" } } });
add("metadata-long-title", "x", { metadata: { title: "t".repeat(3000) } });

// ---- image handling
add("images-max-one", "![a](" + png1x1 + ")\n\n![b](" + png1x1 + ")\n\ninline ![c](" + png1x1 + ")", { imageHandling: { maxImages: 1 } });
add("images-data-disabled", "![a](" + png1x1 + ")", { imageHandling: { dataUrls: { enabled: false } } });
add("images-max-bytes", "![a](" + png1x1 + ")", { imageHandling: { maxImageBytes: 10 } });
add("images-remote-disabled", "![a](https://example.com/a.png)\n\n![b](http://example.com/b.png)");
add("images-remote-enabled-badhost", "![a](https://nonexistent.invalid/a.png)", { imageHandling: { remote: { enabled: true, allowedHosts: ["nonexistent.invalid"] }, fetchTimeoutMs: 500 } });
add("images-hints", "![a](" + png1x1 + "#50x40)\n\n![b](" + png1x1 + "#w=30)\n\n![c](" + png1x1 + "#h=20&w=10)\n\n![d](" + png1x1 + "#99999x1)\n\n![e](" + png1x1 + "#bad)");
add("images-alt-title-invalid", "![a\u0001b](" + png1x1 + ")");
add("images-alt-long", "![" + "a".repeat(600) + "](" + png1x1 + ")");
add("images-gif", "![g](data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7)");
add("images-bmp-unsupported", "![b](data:image/bmp;base64,Qk0=)");
add("images-in-heading-and-link", "# ![a](" + png1x1 + ") Title\n\n[![b](" + png1x1 + ")](https://x.y)\n\n**![c](" + png1x1 + ")**");
add("images-in-table", "| a | b |\n|---|---|\n| ![i](" + png1x1 + ") | ![](missing.png) |");
add("images-in-footnote", "x[^1]\n\n[^1]: ![f](" + png1x1 + ") and ![g](missing.png)\n\n    ![h](" + png1x1 + ")");
add("images-task-list", "- [ ] ![a](" + png1x1 + ")\n- [x] ![b](missing.png)\n- ![c](" + png1x1 + ")\n\n  ![d](" + png1x1 + ")\n- text\n\n  ![e](" + png1x1 + ")");
add("images-placeholder-alt", "![](" + png1x1 + ")\n\ninline ![](" + png1x1 + ")", { accessibility: { missingImageAltText: "permissive" } });

// ---- code highlighting variants
add("highlight-whitelist", "```js\nlet a = 1;\n```\n\n```py\nx = 1\n```", { codeHighlighting: { enabled: true, languages: ["python"] } });
add("highlight-no-label", "```js\nlet a = 1;\n```", { codeHighlighting: { enabled: true, showLanguageLabel: false } });
add("highlight-theme-full", "```js\nconst s = \"x\"; // c\nfunction f(a) { return a + 1 }\n```", { codeHighlighting: { enabled: true, theme: { default: "111111", background: "222222", border: "333333", languageLabel: "444444", keyword: "555555", string: "666666", comment: "777777", "title.function": "888888", params: "999999" } } });
add("highlight-aliases", "```ts\nlet a: number = 1;\n```\n\n```TS\nlet b = 2;\n```\n\n```c++\nint x;\n```\n\n```shell\nls -la\n```\n\n```yml\na: 1\n```\n\n```diff\n+ added\n- removed\n```\n\n```markdown\n# h *e*\n```\n\n```xml\n<a b=\"c\">d</a>\n```", { codeHighlighting: { enabled: true } });
add("highlight-tabs-and-nbsp", "```js\n\tif (a) {\n\t\treturn b;  // tabs\n\t}\n   \n```", { codeHighlighting: { enabled: true } });
add("highlight-mermaid-fallback", "```mermaid\ngraph TD; A-->B\n```", { codeHighlighting: { enabled: true }, mermaidRendering: { enabled: true, failureMode: "codeBlock" } });
add("highlight-chart-disabled", "```chart\n{\"type\":\"bar\"}\n```", { codeHighlighting: { enabled: true } });
add("highlight-in-quote-list", "> ```js\n> let a = 1;\n> ```\n\n- ```js\n  let b = 2;\n  ```\n- text\n\n  ```js\n  let c = 3;\n  ```", { codeHighlighting: { enabled: true } });
add("highlight-back-to-back", "```js\na\n```\n```js\nb\n```\n\n```\nc\n```", { codeHighlighting: { enabled: true } });
add("highlight-label-only", "```js\n```\n\n```js\n\n\n```", { codeHighlighting: { enabled: true } });

// ---- rich tables
add("rich-table-basic", "```table\n{\"headers\":[\"H1\",\"H2\"],\"rows\":[[\"**a**\",\"b\"],[{\"markdown\":\"wide\",\"columnSpan\":2}],[{\"markdown\":\"tall\",\"rowSpan\":2},\"x\"],[\"y\"]],\"columnWidths\":[3000,4000]}\n```");
add("rich-table-nested", "```table\n{\"rows\":[[\"outer\",\"````table\\n{\\\"rows\\\":[[\\\"inner\\\"]]}\\n````\"]]}\n```");
add("rich-table-blocks", "```table\n{\"rows\":[[\"# heading\\n\\npara\\n\\n- list\\n- items\",\"> quote\\n\\n```js\\ncode\\n```\"],[\"![i](" + png1x1 + ")\",\"$x^2$ and [l](https://x.y)\"]]}\n```");
add("rich-table-no-header-widths-mismatch", "```table\n{\"rows\":[[\"a\",\"b\"]],\"columnWidths\":[100]}\n```");
add("rich-table-in-footnote", "x[^1]\n\n[^1]: ```table\n    {\"rows\":[[\"a\",\"b\"]]}\n    ```");
add("rich-table-style-widths", "| a | b |\n|---|---|\n| 1 | 2 |", { style: { tableColumnWidths: [2000, 3000], tableLayout: "fixed", tableAllowRowSplit: false, tableHeaderBackground: "ABCDEF", tableCellMargins: { top: 10, left: 20 } } });
add("rich-table-style-widths-mismatch", "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |", { style: { tableColumnWidths: [2000, 3000] } });
add("table-report-type", "| a |\n|---|\n| b |", { documentType: "report" });
add("table-report-type-custom", "| a |\n|---|\n| b |", { documentType: "report", style: { tableHeaderBackground: "123456" } });

// ---- more style option coverage
add("style-inline-code", "`code` and **`bold code`** and [`link code`](https://x.y)\n\n# `h code`\n\n- `list code`\n\n> `quote code`", { style: { inlineCodeSize: 30, inlineCodeColor: "AA00AA", inlineCodeBackground: "00AA00", paragraphSize: 26 } });
add("style-inline-code-paragraph-size", "`code` text", { style: { paragraphSize: 30 } });
add("style-callouts", "> [!NOTE]\n> n\n\n> [!TIP]\n> t\n\n> [!WARNING]\n> w", { style: { calloutStyles: { note: { borderColor: "111111", backgroundColor: "222222", titleColor: "333333", label: "Custom" }, tip: { titleColor: "444444" } } } });
add("style-alignments", "# H\n\n## H2\n\npara\n\n> quote\n\n```\ncode\n```\n\n---\n\n![i](" + png1x1 + ")\n\n> ![q](" + png1x1 + ")", { style: { headingAlignment: "RIGHT", heading2Alignment: "LEFT", paragraphAlignment: "CENTER", blockquoteAlignment: "JUSTIFIED", codeBlockAlignment: "RIGHT" } });
add("style-sizes", "# H1\n\n###### H6\n\npara\n\n- list\n\n> quote\n\n```\ncode\n```\n\n| a |\n|---|\n| b |\n\nx[^1]\n\n[^1]: note", { style: { heading1Size: 50, heading6Size: 10, paragraphSize: 18, listItemSize: 16, codeBlockSize: 14, blockquoteSize: 12, titleSize: 60 } });
add("style-font-typo-alias", "para `code`\n\n# H", { style: { fontFamilly: "Arial" } });
add("style-font-both", "para", { style: { fontFamily: "", fontFamilly: "Arial" } });
add("style-rtl-everything", "# H\n\npara [l](https://x.y)\n\n- list\n  - nested\n\n1. one\n\n> quote\n\n```\ncode\n```\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n\n![i](" + png1x1 + ")\n\nx[^1]\n\n[^1]: note\n\n---\n\n$x^2$\n\n> [!NOTE]\n> callout", { style: { direction: "RTL", language: "he" }, toc: { enabled: true } });
add("style-line-spacing-odd", "para\n\n# H", { style: { lineSpacing: 1.17, paragraphSpacing: 101, headingSpacing: 241 } });
add("style-spacing-zero", "para\n\n# H\n\n- l\n\n> q\n\n---", { style: { paragraphSpacing: 0, headingSpacing: 0 } });

// ---- math variants
add("math-block-forms", "$$\na^2\n$$\n\n$$b^2$$\n\ntext $$c^2$$ text\n\n$$\n\\begin{aligned} x &= 1 \\\\ y &= 2 \\end{aligned}\n$$\n\n$$ \\sum_{i=1}^{n} i $$");
add("math-inline-edge", "$a$$b$ and $$ and $ $ and \\$5 and $x\\$y$ and `$code$`");
add("math-commands", "$\\frac{a}{b} \\sqrt[3]{x} \\overline{y} \\underline{z} \\vec{v} \\dot{a} \\ddot{b} \\tilde{c} \\mathrm{d} \\mathit{e} \\mathbb{R} \\mathcal{L} \\left[ x \\right] \\left\\{ y \\right\\} \\binom{n}{k} \\lim_{x \\to 0} \\prod_{i} \\int_a^b \\partial \\nabla \\alpha \\Gamma \\pi \\infty \\pm \\mp \\times \\div \\leq \\geq \\neq \\approx \\equiv \\in \\notin \\subset \\cup \\cap \\forall \\exists \\rightarrow \\Rightarrow \\leftrightarrow \\ldots \\cdots \\quad \\, \; \\! a_{i,j}^{k} x^{y^{z}} \\text{plain text} \\textbf{bold} \\operatorname{sin} \\sin \\cos \\log \\ln \\exp \\max \\min$");
add("math-in-table-heading-list", "# $x^2$\n\n| $a$ |\n|---|\n| $b$ |\n\n- $c$\n\n> $d$\n\n[$e$](https://x.y)");
add("math-disabled-dollars", "$ not math $, $$ nope $$", { mathRendering: { enabled: false } });

// ---- footnotes variants
add("footnotes-order", "b[^2] a[^1] c[^3] a[^1]\n\n[^1]: one\n[^2]: two\n[^3]: three");
add("footnotes-labels", "x[^Label] y[^label] z[^ spaced ]\n\n[^label]: lower\n[^ spaced ]: sp");
add("footnotes-in-heading-list-quote-table", "# h[^1]\n\n- l[^1]\n\n> q[^1]\n\n| t[^1] |\n|---|\n| c[^1] |\n\n[^1]: note");
add("footnotes-rich", "x[^1]\n\n[^1]: **bold** `code` [l](https://x.y) $m$ ![i](" + png1x1 + ") ~~s~~ ++u++\n\n    # heading in note\n\n    1. list\n\n    > quote\n\n    ---");
add("footnotes-undefined-and-unused", "a[^x] b[^y]\n\n[^y]: y\n[^z]: z");
add("footnotes-empty-def", "a[^1]\n\n[^1]:");

// ---- html & misc
add("html-blocks-variants", "<!-- COMMENT: first -->\n\n<!--COMMENT:second-->\n\n<!-- comment: lower -->\n\n<!-- COMMENT: multi\nline -->\n\n<div class=\"pagebreak\"></div>\n\n<PAGEBREAK/>\n\n<hr>\n\n<script>alert(1)</script>\n\n<p>\nmulti\nline\n</p>\n\n> <!-- COMMENT: in quote -->\n\n- <!-- COMMENT: in list -->\n- <!-- pagebreak -->");
add("pagebreak-variants", "\\pagebreak\n\n  \\pagebreak  \n\n\\pagebreak extra\n\n> \\pagebreak\n\n- \\pagebreak");
add("entities-and-escapes-in-code", "`&amp; \\* &lt;`\n\n```\n&amp; \\* &lt; <tag>\n```\n\n&amp; \\* &lt; <tag>");
add("links-variants", "[a](https://x.y \"t\") [b](<https://x.y/with space>) [c](https://x.y/%20) [d](javascript:alert(1)) [e](file:///etc/passwd) [f](tel:123) [g](mailto:a@b.c) [h](ftp://x.y) [i](//x.y) [j](/rel) [k](rel) [l](?q) [m](#) [n]() [o](<>) [p](data:text/html,x) [q](HTTPS://X.Y) [r](ht tp://x) [s](https://x.y/\\)) [t](https://x.y/ü)");
add("links-nested-formatting", "[**bold** *em* `code` ~~s~~ ++u++ ![i](" + png1x1 + ")](https://x.y) **[in bold](https://x.y)** [a [b] c](https://x.y) [a](https://1.y) [b](https://1.y)");
add("autolinks-edge", "www.a.b www.a.b. www.a.b, (www.a.b) [www.a.b] *www.a.b* _www.a.b_ ~www.a.b~ xwww.a.b www.a_b.c www.a.b_c www.a.b/_c www.a.b/c_ http://a.b/?x=1&y=2 https://a.b/#frag HTTP://A.B http://a b http://a.b</p> http://a.b&amp; http://a.b&foo;bar http://a.b&quot; a@b.c. a@b.c, (a@b.c) a@b. a@b.c-d a@b._c a.b@c.d/e /a@b.c mailto:a@b.c xmpp:a@b.c https://a.b/(c) https://a.b/(c)) https://a.b/c) www.點看.com https://ü.de");

// ---- reference and patch paths
add("reference-basic", "# Title\n\nParagraph with **bold** and [link](https://x.y).\n\n- a\n- b\n\n1. x\n2. y\n\n> quote\n\n```js\ncode\n```\n\n| h |\n|---|\n| c |\n\nx[^1]\n\n[^1]: note", {}, { kind: "reference", reference });
add("reference-options", "# Title\n\npara", { reference: { preservePageLayout: false }, style: { paragraphSize: 26 }, metadata: { title: "Ref" } }, { kind: "reference", reference });
add("reference-style-map", "# Title\n\npara `code`\n\n> q", { reference: { styles: { heading1: { id: "Heading1" }, normal: { name: "Normal" }, missing: "ignore" } } }, { kind: "reference", reference });
add("reference-empty", "", {}, { kind: "reference", reference });
add("patch-basic", "", {}, { kind: "patch", reference, patches: { body: "# Patched\n\nText **bold**", summary: "- one\n- two" } });
add("patch-multiple-headings", "", {}, { kind: "patch", reference, patches: { body: "# Technical report\n\n## Executive summary\n\nText\n\n## Methodology\n\n1. Collect data\n2. Analyze\n\n## Results\n\n| Metric | Value |\n|---|---|\n| Users | 1240 |\n\n## Recommendations\n\n- Ship\n\n## Conclusion\n\nDone." } });
add("patch-missing-placeholder", "", {}, { kind: "patch", reference, patches: { nothere: "x" } });
add("patch-footnotes-toc", "", { toc: { enabled: true } }, { kind: "patch", reference, patches: { body: "[TOC]\n\n# H\n\nnote[^1]\n\n[^1]: f" } });
add("patch-comment-rejected", "", {}, { kind: "patch", reference, patches: { body: "<!-- COMMENT: c -->" } });
add("patch-images", "", {}, { kind: "patch", reference, patches: { body: "![i](" + png1x1 + ")" } });
add("patch-empty-patches", "", {}, { kind: "patch", reference, patches: {} });
add("patch-empty-markdown", "", {}, { kind: "patch", reference, patches: { body: "" } });
add("patch-delimiters", "", { placeholderDelimiters: { start: "[[", end: "]]" } }, { kind: "patch", reference, patches: { body: "x" } });
add("patch-table-width", "", { tableWidthTwips: 5000 }, { kind: "patch", reference, patches: { body: "| a |\n|---|\n| b |" } });
add("patch-metadata", "", { metadata: { title: "Patched", custom: { K: "V" } } }, { kind: "patch", reference, patches: { body: "x" } });
add("patch-limits", "", { maxElements: 2 }, { kind: "patch", reference, patches: { body: "# a\n\nb\n\nc" } });

// ---- seeded random documents mixing constructs
function rng(seed) { let s = seed >>> 0; return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296; }; }
const inlineAtoms = ["text", "**b**", "*i*", "`c`", "~~s~~", "++u++", "[l](https://x.y/p)", "![i](" + png1x1 + ")", "x[^1]", "$m^2$", "www.a.b", "a@b.c", "<b>h</b>", "&amp;", "\\*", "line  \nbreak", "soft\nbreak", "H~2~O", "a*b*c", "a_b_c", "__s__", "***bi***", "[ref][r]", "<https://z.q>", "\"quote\"", "—", "日本", "[@fig:a]", "{#x}", "[]", "()", "##", "1.", "- not list", "> not quote"];
const blockAtoms = [
  () => "# H1", () => "## H2 **b**", () => "### H3 `c`", () => "Setext\n===", () => "Setext2\n---",
  () => "- a\n- b\n  - c", () => "1. a\n2. b\n   1. c", () => "- [ ] t\n- [x] d", () => "* a\n\n* b\n\n  para", () => "3) a\n4) b",
  () => "> q\n> q2", () => "> > nested", () => "> [!NOTE]\n> n", () => "> [!TIP] inline", () => "> - l\n> - m",
  () => "```js\nlet a = 1;\n```", () => "```\nplain\n```", () => "    indented", () => "~~~\ntilde\n~~~", () => "```table\n{\"rows\":[[\"a\",\"b\"]]}\n```",
  () => "| a | b |\n|---|--:|\n| 1 | 2 |", () => "| x |\n|---|", () => "a | b\n--|--\n1 | 2",
  () => "---", () => "***", () => "![img](" + png1x1 + ")", () => "![missing](nope.png)", () => "$$\nx^2\n$$", () => "$$ y $$",
  () => "[^1]: footnote **b**", () => "[r]: https://r.ef", () => "<!-- COMMENT: c -->", () => "<!-- pagebreak -->", () => "<div>html</div>", () => "\\pagebreak", () => "[TOC]",
  () => "Figure: cap {#fig:a}", () => "Table: cap {#tbl:a}", () => "Term\n: Def", () => "+++\nfm\n+++", () => "",
];
for (let seed = 1; seed <= 150; seed++) {
  const r = rng(seed * 7919);
  const blocks = [];
  const count = 2 + Math.floor(r() * 7);
  for (let i = 0; i < count; i++) {
    if (r() < 0.45) {
      const n = 1 + Math.floor(r() * 5);
      const parts = [];
      for (let j = 0; j < n; j++) parts.push(inlineAtoms[Math.floor(r() * inlineAtoms.length)]);
      blocks.push(parts.join(r() < 0.5 ? " " : ""));
    } else {
      blocks.push(blockAtoms[Math.floor(r() * blockAtoms.length)]());
    }
  }
  const sep = r() < 0.15 ? "\n" : "\n\n";
  const md = blocks.join(sep) + (r() < 0.5 ? "\n" : "");
  const opts = r() < 0.3 ? { codeHighlighting: { enabled: true }, toc: { enabled: true } } : r() < 0.5 ? { style: { direction: "RTL", paragraphSize: 22 } } : undefined;
  add(`rand-${String(seed).padStart(3, "0")}`, md, opts);
}

export default cases;
