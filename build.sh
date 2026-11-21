#!/usr/bin/env bash
# Dựng gateway Rust trên máy dựng trong container rust tạm, giới hạn CPU để không làm chậm HA.
# Đặt TGGW_BUILD_HOST thành SSH alias của máy dựng (mặc định: builder).
# Dùng: ./build.sh           (cargo build --release → binary ở /opt/telegram-gateway-rs-build/target/release/tggw)
#       ./build.sh test      (cargo test)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
DEST=${TGGW_BUILD_DIR:-/opt/telegram-gateway-rs-build}   # mỗi worktree/agent một thư mục riêng
RUST_IMG=tggw-builder:1.90
CMD=${1:-build}
REMOTE_HOST=${TGGW_BUILD_HOST:-}
if [[ -z "$REMOTE_HOST" && -f "$HERE/.tggw-build-host" ]]; then
  IFS= read -r REMOTE_HOST < "$HERE/.tggw-build-host" || [[ -n "$REMOTE_HOST" ]]
fi
REMOTE_HOST=${REMOTE_HOST:-builder}
ssh "$REMOTE_HOST" "docker image inspect $RUST_IMG >/dev/null 2>&1" || ssh "$REMOTE_HOST" "docker build -q -t $RUST_IMG -" < "$HERE/docker/builder.Dockerfile"
case "$CMD" in
  build) ARGS="cargo build --release --locked 2>&1 || cargo build --release 2>&1" ;;
  test)  ARGS="cargo test --release 2>&1" ;;
  check) ARGS="cargo clippy --release --all-targets 2>&1 || cargo check --release --all-targets 2>&1" ;;
  *) echo "lệnh lạ: $CMD"; exit 64 ;;
esac
# Khoá cả chép mã lẫn dựng; -m đặt mtime lúc giải nén để Cargo không dùng artefact của snapshot khác.
tar -C "$HERE" --exclude=./target --exclude=./.git --exclude='./.h2-*' --exclude=./.tggw-build-host -cf - . | \
  ssh "$REMOTE_HOST" "mkdir -p '$DEST' && flock /tmp/tggw-build.lock sh -c 'tar -m -C \"$DEST\" -xf - && exec docker run --rm --cpus 3 -v \"$DEST:/src\" -w /src -v tggw-cargo-registry:/usr/local/cargo/registry $RUST_IMG sh -c \"\$1\"' -- '$ARGS'"
