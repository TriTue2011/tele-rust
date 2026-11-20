#!/usr/bin/env bash
# Dựng gateway Rust TRÊN .200 trong container rust tạm (không cài rustup lên máy nào), giới hạn CPU để không làm chậm HA.
# Dùng: ./build.sh           (chép mã sang .200 → cargo build --release → binary ở /opt/telegram-gateway-rs-build/target/release/tggw)
#       ./build.sh test      (cargo test)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
DEST=${TGGW_BUILD_DIR:-/opt/telegram-gateway-rs-build}   # mỗi worktree/agent một thư mục riêng
RUST_IMG=tggw-builder:1.90
CMD=${1:-build}
ssh ha200 "docker image inspect $RUST_IMG >/dev/null 2>&1" || ssh ha200 "docker build -q -t $RUST_IMG -" < "$HERE/docker/builder.Dockerfile"
tar -C "$HERE" --exclude=./target --exclude=./.git -cf - . | ssh ha200 "mkdir -p $DEST && tar -C $DEST -xf -"
case "$CMD" in
  build) ARGS="cargo build --release --locked 2>&1 || cargo build --release 2>&1" ;;
  test)  ARGS="cargo test --release 2>&1" ;;
  check) ARGS="cargo clippy --release --all-targets 2>&1 || cargo check --release --all-targets 2>&1" ;;
  *) echo "lệnh lạ: $CMD"; exit 64 ;;
esac
# Xếp hàng: mỗi lúc chỉ MỘT bản dựng trên .200 (máy HA 8 nhân), tối đa 3 nhân.
ssh ha200 "flock /tmp/tggw-build.lock docker run --rm --cpus 3 -v $DEST:/src -w /src -v tggw-cargo-registry:/usr/local/cargo/registry \
  $RUST_IMG sh -c '$ARGS'"
