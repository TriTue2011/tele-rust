#!/usr/bin/env bash
# Dựng gateway Rust TRÊN .200 trong container rust tạm (không cài rustup lên máy nào), giới hạn CPU để không làm chậm HA.
# Dùng: ./build.sh           (chép mã sang .200 → cargo build --release → binary ở /opt/telegram-gateway-rs-build/target/release/tggw)
#       ./build.sh test      (cargo test)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
DEST=/opt/telegram-gateway-rs-build
RUST_IMG=rust:1.86-slim-bookworm
CMD=${1:-build}
tar -C "$HERE" --exclude=./target --exclude=./.git -cf - . | ssh ha200 "mkdir -p $DEST && tar -C $DEST -xf -"
case "$CMD" in
  build) ARGS="cargo build --release --locked 2>&1 || cargo build --release 2>&1" ;;
  test)  ARGS="cargo test --release 2>&1" ;;
  *) echo "lệnh lạ: $CMD"; exit 64 ;;
esac
ssh ha200 "docker run --rm --cpus 3 -v $DEST:/src -w /src -v tggw-cargo-registry:/usr/local/cargo/registry \
  $RUST_IMG sh -c '$ARGS' | tail -25; ls -la $DEST/target/release/tggw 2>/dev/null"
