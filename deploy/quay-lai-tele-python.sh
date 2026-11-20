#!/usr/bin/env bash
# Quay lại gateway Telegram Python: dừng bản Rust (KHÔNG xoá), khởi động lại Python. Từ chối khi đang có cuộc gọi trừ khi đặt BO_QUA_CUOC_GOI=1.
set -euo pipefail
ssh ha200 "BO_QUA=${BO_QUA_CUOC_GOI:-0} bash -s" <<'REMOTE'
set -euo pipefail
PY=telegram-gateway; RS=telegram-gateway-rs
TOKEN=$(docker inspect "$RS" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^GATEWAY_TOKEN=//p')
HOST=$(docker inspect "$RS" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^GATEWAY_HOST=//p')
PORT=$(docker inspect "$RS" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^REST_PORT=//p')
if [ "${BO_QUA:-0}" != "1" ]; then
  N=$(python3 - "$HOST" "$PORT" "$TOKEN" <<'PY' 2>/dev/null || echo x
import json,sys,urllib.request
r=urllib.request.Request("http://%s:%s/api/live/calls"%(sys.argv[1],sys.argv[2]),headers={"Authorization":"Bearer "+sys.argv[3]})
print(len(json.loads(urllib.request.urlopen(r,timeout=10).read()).get("calls",[])))
PY
)
  [ "$N" = "0" ] || [ "$N" = "x" ] || { echo "TỪ CHỐI: đang có $N cuộc gọi"; exit 3; }
fi
docker update --restart no "$RS" >/dev/null
docker stop "$RS" >/dev/null; echo "đã dừng $RS"
docker start "$PY" >/dev/null; docker update --restart unless-stopped "$PY" >/dev/null
echo "XONG: Python đang phục vụ lại"
REMOTE
