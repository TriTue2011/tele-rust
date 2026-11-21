#!/usr/bin/env bash
# Thay container Rust ĐANG CHẠY (telegram-gateway-rs) bằng bản chạy trong ảnh CÓ ffmpeg/edge-tts (telegram-gateway:local-200)
# và nhị phân mới dựng ở /opt/tggw-build-tichhop. Không xoá gì: container Rust cũ chỉ bị dừng + đổi tên.
# Từ chối khi đang có cuộc gọi ĐANG DIỄN RA (cuộc đã kết thúc không tính). Chờ /health online; không đạt thì quay lại Python.
set -euo pipefail
ssh "${TGGW_BUILD_HOST:-builder}" "TGGW_NEWBIN=${TGGW_NEWBIN:-}" bash -s <<'REMOTE'
set -euo pipefail
RS=telegram-gateway-rs; PY=telegram-gateway; BASE=/opt/telegram-gateway-rs; NEWBIN=${TGGW_NEWBIN:-/opt/tggw-build-tichhop/target/release/tggw}
IMG=telegram-gateway:local-200
env_of() { docker inspect "$RS" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n "s/^$1=//p"; }
TOKEN=$(env_of GATEWAY_TOKEN); HOST=$(env_of GATEWAY_HOST); PORT=$(env_of REST_PORT)
API_ID=$(env_of TG_API_ID); API_HASH=$(env_of TG_API_HASH); PHONE=$(env_of TG_PHONE)
SESSION=$(docker inspect "$RS" --format '{{range .Mounts}}{{if eq .Destination "/data/tdlib"}}{{.Source}}{{end}}{{end}}')
[ -n "$TOKEN" ] && [ -n "$HOST" ] && [ -n "$PORT" ] && [ -n "$SESSION" ] || { echo "thiếu cấu hình của container Rust hiện tại"; exit 2; }
[ -x "$NEWBIN" ] || { echo "không thấy nhị phân mới $NEWBIN"; exit 2; }
# 1. Từ chối khi có cuộc gọi đang diễn ra.
ACTIVE=$(python3 - "$HOST" "$PORT" "$TOKEN" <<'PY'
import json,sys,urllib.request
r=urllib.request.Request("http://%s:%s/api/live/calls"%(sys.argv[1],sys.argv[2]),headers={"Authorization":"Bearer "+sys.argv[3]})
calls=json.loads(urllib.request.urlopen(r,timeout=10).read()).get("calls",[])
print(sum(1 for c in calls if c.get("state") not in ("ended","failed")))
PY
)
[ "$ACTIVE" = "0" ] || { echo "TỪ CHỐI: đang có $ACTIVE cuộc gọi diễn ra, thử lại sau"; exit 3; }
# 2. Đặt nhị phân mới (cp ra tệp tạm rồi mv để không đè tệp đang chạy).
cp "$NEWBIN" "$BASE/tggw.new" && chmod 755 "$BASE/tggw.new" && mv -f "$BASE/tggw.new" "$BASE/tggw"
# 3. Dừng + đổi tên container cũ (không xoá), chạy bản mới trong ảnh có ffmpeg.
docker stop "$RS" >/dev/null; docker update --restart no "$RS" >/dev/null
docker rename "$RS" "$RS-cu-$(date +%Y%m%d-%H%M%S)"
TDLIB=$(ls "$BASE"/tdjson.libs/libtdjson-*.so* | head -1 | sed "s#$BASE#/opt/rs#")
docker run -d --name "$RS" --network host --restart no --entrypoint /opt/rs/tggw \
  --health-cmd "python -c \"import urllib.request; urllib.request.urlopen('http://$HOST:$PORT/health', timeout=4)\"" \
  --health-interval 30s --health-timeout 5s --health-retries 3 --health-start-period 40s \
  -v "$BASE:/opt/rs:ro" -v "$BASE/data:/data" -v "$SESSION:/data/tdlib" -w /data \
  -e TDJSON_LIB="$TDLIB" -e TGGW_DATA=/data -e TG_API_ID="$API_ID" -e TG_API_HASH="$API_HASH" -e TG_PHONE="$PHONE" \
  -e GATEWAY_TOKEN="$TOKEN" -e GATEWAY_HOST="$HOST" -e REST_PORT="$PORT" \
  "$IMG" serve >/dev/null
# 4. Chờ /health online (tối đa 90 giây); không đạt thì quay lại Python.
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
  echo "KHÔNG ĐẠT: bản mới không online sau 90 giây — QUAY LẠI Python"
  docker stop "$RS" >/dev/null 2>&1 || true
  docker start "$PY" >/dev/null; docker update --restart unless-stopped "$PY" >/dev/null; echo "đã khởi động lại $PY"; exit 4
fi
# 5. Kiểm môi trường chạy có đủ công cụ ngoài, rồi chốt khởi động cùng máy.
docker exec "$RS" sh -c 'which ffmpeg ffprobe edge-tts' | sed 's/^/công cụ trong ảnh chạy: /'
python3 - "$HOST" "$PORT" "$TOKEN" "$PHONE" <<'PY'
import sys,urllib.request,urllib.parse
h,p,t,ph=sys.argv[1:5]
for path in ["/api/accounts","/api/me?account="+urllib.parse.quote(ph),"/api/live/calls","/api/feed","/api/events?since=0"]:
    r=urllib.request.Request("http://%s:%s%s"%(h,p,path),headers={"Authorization":"Bearer "+t})
    print(path.split("?")[0],"->",urllib.request.urlopen(r,timeout=15).status)
PY
docker update --restart unless-stopped "$RS" >/dev/null
echo "XONG: bản Rust mới (ảnh có ffmpeg) đang phục vụ. Cuộc gọi Tele sẵn sàng để thử."
REMOTE
