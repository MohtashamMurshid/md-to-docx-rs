#!/usr/bin/env bash
# Run from any directory. npm ci supplies the pinned TypeScript parity oracle.
set -euo pipefail
workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace"
npm ci --prefix crates/md-to-docx/tests/oracle
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
npm ci --prefix crates/md-to-docx-napi
npm run build --prefix crates/md-to-docx-napi
npm test --prefix crates/md-to-docx-napi
npm run test:types --prefix crates/md-to-docx-napi
python3 -I -m unittest discover -s scripts/parity -p 'test_*.py'
