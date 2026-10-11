# md-to-docx-rs

Rust port of [@mohtasham/md-to-docx](https://github.com/MohtashamMurshid/md-to-docx).

## Build

The toolchain is pinned in `rust-toolchain.toml` (currently `1.99.0`). rustup selects that toolchain on the first build.

```sh
cargo build --workspace
```

## Crates

- `crates/md-to-docx` — library that turns Markdown into a Word `.docx` package.
- `crates/md-to-docx-cli` — command-line binary named `md-to-docx`.
- `crates/md-to-docx-napi` — Node-API binding around the conversion API.
