#!/usr/bin/env bash
# Chạy từ gốc worktree, chỉ dùng container tạm và thư mục dựng riêng.
set -euo pipefail
exec 9<"$0"
flock 9
BUILD_DIR=${TGGW_BUILD_DIR:-/opt/tggw-build-h2}
ssh ha200 "test -d '$BUILD_DIR/tests/contract' && docker run --rm -v '$BUILD_DIR:/src' -w /src tggw-builder:1.90 gcc -shared -fPIC tests/contract/tdjson_rong.c -o libtdjson-rong.so"
cleanup() { ssh ha200 'docker stop -t 15 tggw-h2-thu' >/dev/null; }
trap cleanup EXIT
ssh ha200 "docker run -d --rm --name tggw-h2-thu --network host -v '$BUILD_DIR:/src:ro' --tmpfs /data:mode=0700 -e GATEWAY_HOST=127.0.0.1 -e REST_PORT=3399 -e GATEWAY_TOKEN=thu -e TG_API_ID=12345 -e TG_API_HASH=fake-api-hash -e TG_PHONE=+84900000000 -e TDJSON_LIB=/src/libtdjson-rong.so tggw-builder:1.90 sh -c 'mkdir -p /data/tdlib/files && exec /src/target/release/tggw serve'"
ssh ha200 "python3 -c 'import time,urllib.request
for attempt in range(100):
 try:
  urllib.request.urlopen(\"http://127.0.0.1:3399/health\",timeout=1); break
 except OSError: time.sleep(0.1)
else: raise SystemExit(\"HTTP không khởi động\")'"
ssh ha200 "python3 '$BUILD_DIR/tests/contract/kiem_hop_dong.py' http://127.0.0.1:3399 thu"
ssh ha200 "docker logs tggw-h2-thu 2>&1" | python3 -c 'import re,sys; data=sys.stdin.read(); assert not re.search(r"\+84900000000|secret2fa|qr-test-secret|fake-api-hash|54321|22222",data), "Log chứa dữ liệu đăng nhập"; print("Quét log hộp đen: 0 dữ liệu nhạy cảm")'
