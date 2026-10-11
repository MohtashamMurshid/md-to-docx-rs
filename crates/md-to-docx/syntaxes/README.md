# TypeScript syntax definitions

`TypeScript.sublime-syntax` and `TypeScriptReact.sublime-syntax` provide real
TypeScript and TSX tokenization through syntect. They are bundled so highlighting
does not need network access or a local editor installation.

The definitions were downloaded from [bat](https://github.com/sharkdp/bat/tree/d9559c69f541873f8c47ac8ad81d985f4441edf6/assets/syntaxes/02_Extra)
at commit `d9559c69f541873f8c47ac8ad81d985f4441edf6`. The TSX source filename
there is `TypsecriptReact.sublime-syntax`. bat converts these definitions from
[Microsoft TypeScript-Sublime-Plugin](https://github.com/microsoft/TypeScript-Sublime-Plugin/tree/ba45efd058df5111837e30fb9598cfc8cbd51095),
pinned by bat at commit `ba45efd058df5111837e30fb9598cfc8cbd51095`.

Upstream is licensed under Apache-2.0. Its license is retained in
`LICENSE.typescript`. The grammar files are otherwise unchanged.

Supported fence names are `ts`, `typescript`, `mts`, `cts`, `tsx`,
`typescriptreact`, and `typescript-react`, ignoring case and outer whitespace.
The existing theme maps the grammar's scopes to colors. Those colors and token
boundaries need not match lowlight's TypeScript grammar exactly. Unknown languages
still use plain code. Syntax definitions initialize once per process, and adjacent
text with the same color shares one span.

The earlier 7.15 ms highlighted-TypeScript benchmark used plain fallback code.
It does not measure this implementation and must be rerun before claiming a
highlighted-code speedup.
