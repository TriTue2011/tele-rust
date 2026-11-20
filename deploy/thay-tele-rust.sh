#!/usr/bin/env bash
# Thay gateway Telegram Python (container `telegram-gateway`) bằng bản Rust (`telegram-gateway-rs`) trên .200.
# KHÔNG xoá gì: Python chỉ bị DỪNG (docker stop), có thể quay lại bằng quay-lai-tele-python.sh.
# Từ chối khi đang có cuộc gọi. Sao lưu phiên TDLib trước. Chờ /health báo tài khoản online; không đạt thì TỰ quay lại Python.
# Cần: ssh ha200; nhị phân + libtdjson đã đặt ở /opt/telegram-gateway-rs/{tggw,tdjson.libs}.
set -euo pipefail
ssh ha200 'bash -s' <<'REMOTE'
set -euo pipefail
PY=telegram-gateway; RS=telegram-gateway-rs
BASE=/opt/telegram-gateway-rs; DATA_PY=/opt/telegram-gateway/data
TOKEN=$(docker inspect "$PY" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^GATEWAY_TOKEN=//p')
HOST=$(docker inspect "$PY" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^GATEWAY_HOST=//p')
PORT=$(docker inspect "$PY" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^REST_PORT=//p')
API_ID=$(docker inspect "$PY" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^TELEGRAM_API_ID=//p')
API_HASH=$(docker inspect "$PY" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^TELEGRAM_API_HASH=//p')
SESSION=$(ls -d "$DATA_PY"/tdlib/*/ | while read -r d; do [ -f "$d/tai_khoan.json" ] && echo "${d%/}"; done | head -1)
[ -n "$SESSION" ] && [ -n "$TOKEN" ] && [ -n "$HOST" ] && [ -n "$PORT" ] || { echo "thiếu cấu hình Python (phiên/token/host/cổng)"; exit 2; }
PHONE=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))[0]["phone"])' "$DATA_PY/accounts.json")
# 1. Từ chối khi đang có cuộc gọi.
CALLS=$(python3 - "$HOST" "$PORT" "$TOKEN" <<'PY'
import json,sys,urllib.request
r=urllib.request.Request("http://%s:%s/api/live/calls"%(sys.argv[1],sys.argv[2]),headers={"Authorization":"Bearer "+sys.argv[3]})
print(len(json.loads(urllib.request.urlopen(r,timeout=10).read()).get("calls",[])))
PY
)
[ "$CALLS" = "0" ] || { echo "TỪ CHỐI: đang có $CALLS cuộc gọi, thử lại sau"; exit 3; }
# 2. Sao lưu phiên TDLib (cp, không xoá).
SL="$DATA_PY/sao-luu-tdlib-$(date +%Y%m%d-%H%M%S)"; install -d -m 700 "$SL"; cp -a "$SESSION" "$SL/"; echo "đã sao lưu phiên vào $SL"
install -d -m 700 "$BASE/data"
# 3. Dừng Python (KHÔNG xoá) rồi mới chạy Rust — không bao giờ chạy song song cùng một tài khoản.
docker stop "$PY" >/dev/null; echo "đã dừng $PY"
if docker inspect "$RS" >/dev/null 2>&1; then docker stop "$RS" >/dev/null 2>&1 || true; docker rename "$RS" "$RS-cu-$(date +%Y%m%d-%H%M%S)"; fi   # KHÔNG docker rm: đổi tên bản cũ
docker run -d --name "$RS" --network host --restart no --entrypoint /opt/rs/tggw \
  -v "$BASE:/opt/rs:ro" -v "$BASE/data:/data" -v "$SESSION:/data/tdlib" -w /data \
  -e TDJSON_LIB=$(ls /opt/telegram-gateway-rs/tdjson.libs/libtdjson-*.so* | head -1 | sed 's#/opt/telegram-gateway-rs#/opt/rs#') \
  -e TGGW_DATA=/data -e TG_API_ID="$API_ID" -e TG_API_HASH="$API_HASH" -e TG_PHONE="$PHONE" \
  -e GATEWAY_TOKEN="$TOKEN" -e GATEWAY_HOST="$HOST" -e REST_PORT="$PORT" \
  telegram-gateway:local-200 serve >/dev/null
# 4. Chờ /health báo tài khoản online (tối đa 90 giây); không đạt thì quay lại Python.
OK=0
for i in $(seq 1 45); do
  sleep 2
  if python3 - "$HOST" "$PORT" "$TOKEN" <<'PY' 2>/dev/null
import json,sys,urllib.request
r=urllib.request.Request("http://%s:%s/health"%(sys.argv[1],sys.argv[2]),headers={"Authorization":"Bearer "+sys.argv[3]})
d=json.loads(urllib.request.urlopen(r,timeout=5).read())
sys.exit(0 if d.get("accounts_online",0)>=1 else 1)
PY
  then OK=1; break; fi
done
if [ "$OK" != "1" ]; then
  echo "KHÔNG ĐẠT: bản Rust không online sau 90 giây — QUAY LẠI Python"
  docker stop "$RS" >/dev/null 2>&1 || true
  docker start "$PY" >/dev/null; echo "đã khởi động lại $PY"; exit 4
fi
# 5. Kiểm nhanh đường HA hay gọi (chỉ đọc), rồi chốt cấu hình khởi động.
python3 - "$HOST" "$PORT" "$TOKEN" "$PHONE" <<'PY'
import json,sys,urllib.request,urllib.parse
h,p,t,ph=sys.argv[1:5]
for path in ["/api/accounts","/api/me?account="+urllib.parse.quote(ph),"/api/live/calls","/api/feed","/api/events?since=0"]:
    r=urllib.request.Request("http://%s:%s%s"%(h,p,path),headers={"Authorization":"Bearer "+t})
    x=urllib.request.urlopen(r,timeout=15); print(path.split("?")[0],"->",x.status)
PY
docker update --restart unless-stopped "$RS" >/dev/null
docker update --restart no "$PY" >/dev/null
echo "XONG: Rust đang phục vụ thay Python. Quay lại bằng quay-lai-tele-python.sh"
REMOTE
