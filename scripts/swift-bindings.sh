#!/usr/bin/env sh
# Generates the Swift bindings for almond-ffi into ./generated/swift.
set -eu
cd "$(dirname "$0")/.."
cargo build -p almond-ffi
case "$(uname)" in
  Darwin) lib=target/debug/libalmond_ffi.dylib ;;
  *) lib=target/debug/libalmond_ffi.so ;;
esac
cargo run -q -p almond-ffi --bin uniffi-bindgen -- \
  generate --library "$lib" --language swift --out-dir generated/swift
