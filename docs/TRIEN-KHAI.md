# Hướng dẫn triển khai — tele-rust (`v0.7.49`)

## 1. Yêu cầu hệ thống

- **Kiến trúc hỗ trợ:** `linux/amd64` và `linux/arm64` (ảnh đa kiến trúc, dựng qua QEMU — NTgCalls `3.0.7` và wheel `tdjson==1.8.69` đều có bản arm64).
  - **macOS native** (ứng dụng Mac chạy trực tiếp, không qua container) chưa được hỗ trợ — khác với kiến trúc CPU `arm64`: ảnh `linux/arm64` vẫn chạy được trên Apple Silicon qua Docker Desktop/Raspberry Pi, chỉ không có app Mac riêng.
- **Thông số Telegram bắt buộc:** `TG_API_ID` và `TG_API_HASH` (đăng ký ứng dụng tại trang phát triển Telegram).
- **Khoá quản trị:** `GATEWAY_TOKEN` (khuyến nghị sinh bằng `openssl rand -hex 32`).

## 2. Triển khai bằng Docker / Docker Compose

Ảnh chính thức trên GitHub Container Registry: `ghcr.io/tritue2011/tele-rust`.

- **Người dùng container:** `tggw` (`UID=10001`, `GID=10001`), không chạy quyền `root`.
- **Thư mục dữ liệu:** `/data` (nên gắn volume bền vững để giữ phiên TDLib, `accounts.json`, `ui_auth.json` và lịch sử nhật ký).
- **Cổng mặc định:** `3334/tcp` (`REST_PORT=3334`, `GATEWAY_HOST=0.0.0.0`).
- **Kiểm tra sức khoẻ (HEALTHCHECK):** gọi `GET /health` mỗi 30 giây.

Khi gắn trực tiếp thư mục máy chủ (bind mount) vào `/data`, cần đặt chủ sở hữu cho UID/GID `10001`:

```bash
mkdir -p ./tele-rust-data
chown 10001:10001 ./tele-rust-data
docker run -d \
  --name tele-rust \
  --restart unless-stopped \
  -p 127.0.0.1:3334:3334 \
  -v "$(pwd)/tele-rust-data:/data" \
  -e TG_API_ID=12345 \
  -e TG_API_HASH='fake-api-hash-placeholder' \
  -e GATEWAY_TOKEN='fake-gateway-token-placeholder' \
  ghcr.io/tritue2011/tele-rust:latest
```

**HTTPS cho mic/webcam của trình duyệt:** cùng cổng đó nhận cả `http://` và `https://` (chứng chỉ tự ký, tự sinh lần đầu và lưu ở `/data/tls/` nên nhớ gắn volume `/data`). Trình duyệt chỉ cho dùng mic và webcam trên HTTPS hoặc `localhost`: mở `https://<máy>:3334`, chấp nhận cảnh báo chứng chỉ một lần. Không cần mở thêm cổng hay đổi `docker-compose`/add-on. Tắt bằng `GATEWAY_HTTPS=0`; thêm tên/IP vào chứng chỉ bằng `GATEWAY_TLS_NAMES`.

Lưu ý: Không công bố trực tiếp cổng `3334` ra mạng công cộng mà không có lớp HTTPS và kiểm soát truy cập. Nên lắng nghe ở `127.0.0.1:3334` và đưa qua reverse proxy.

## 3. Triển khai Add-on Home Assistant

1. Mở **Cài đặt → Tiện ích bổ sung → Cửa hàng tiện ích bổ sung → Kho lưu trữ**.
2. Thêm URL kho: `https://github.com/TriTue2011/tele-rust`.
3. Cài đặt **Tele Rust**, điền tối thiểu `tg_api_id`, `tg_api_hash`, `gateway_token` và bật add-on.
4. Truy cập giao diện web qua cổng `3334` (`http://<ha-host>:3334` hoặc reverse proxy).
   - **Vì sao chưa bật Ingress:** gateway đặt CSP `frame-ancestors 'none'`, tài nguyên web gọi đường dẫn tuyệt đối `/api/*` và cookie đặt `Path=/`, nên không chạy lồng trong iframe Ingress của Home Assistant.

### Khi kho chuyển sang chế độ riêng tư (private)

Nếu chủ kho chuyển `TriTue2011/tele-rust` và package `ghcr.io/tritue2011/tele-rust` sang private:
1. Tạo GitHub Personal Access Token (classic) chỉ với quyền `read:packages`.
2. Trên Home Assistant host, đăng nhập GHCR trước khi cài đặt hoặc cập nhật add-on:
   ```bash
   ha docker registries add ghcr.io --username <github-user> --password <pat>
   ```
3. Với Docker độc lập, chạy `docker login ghcr.io -u <github-user>`.

### Cấu trúc dữ liệu dưới Home Assistant Supervisor

Supervisor gắn `/data/options.json` vào `/data`. Vì module `UiAuth` ép quyền `0700` lên thư mục `TGGW_DATA`, `docker/entrypoint.sh` tự động chuyển `TGGW_DATA` thành `/data/state` khi phát hiện `/data/options.json`, giữ nguyên quyền đọc `options.json` cho entrypoint và bảo vệ tệp `ui_auth.json`, phiên TDLib trong `/data/state`.

## 4. Triển khai sau Reverse Proxy HTTPS

Ví dụ cấu hình Nginx chuyển tiếp HTTPS vào `127.0.0.1:3334`:

```nginx
server {
    listen 443 ssl http2;
    server_name tele.example.test;

    location / {
        proxy_pass http://127.0.0.1:3334;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto https;
        proxy_read_timeout 60s;
    }
}
```

Khi đặt sau proxy HTTPS:
- Bắt buộc đặt `TGGW_UI_COOKIE_SECURE=1` (hoặc `ui_cookie_secure: true` trong add-on) để cookie phiên luôn có thuộc tính `Secure` và yêu cầu CSRF chỉ chấp nhận `Origin: https://...`.
- Tăng `proxy_read_timeout` lên tối thiểu `60s` để không ngắt kết nối long-poll `/api/feed?wait=25`, `/api/live/view/:call_id` và `/api/live/audio/out/:call_id`.

## 5. Tự dựng từ mã nguồn

### Dựng ảnh Docker
```bash
# Dựng đầy đủ kèm edge-tts và TDLib 1.8.69:
docker build --build-arg APP_VERSION=0.7.1 -t tele-rust:local .

# Dựng có kiểm tra SHA256 của wheel tdjson==1.8.69:
docker build --build-arg TDJSON_SHA256=<sha256-wheel> -t tele-rust:verified .

# Bỏ qua edge-tts để ảnh gọn hơn nếu không dùng TTS:
docker build --build-arg WITH_TTS=0 -t tele-rust:no-tts .
```

### Dựng nhị phân trực tiếp bằng Cargo
Yêu cầu Rust `1.90`, `cmake`, `make`, `pkg-config`, `g++`, `zlib1g-dev`, `ffmpeg`:

```bash
cargo build --release --bin tggw
cargo test --release
```

Nếu muốn đóng gói lại với phiên bản NTgCalls khác, tham khảo hướng dẫn liên kết lại LGPL-3.0 trong [NOTICE.md](../NOTICE.md).

## 6. Vận hành, sao lưu và đặt lại mật khẩu web

- **Sao lưu dữ liệu:** Dừng container (`docker stop tele-rust`) trước khi sao lưu toàn bộ `/data` (chứa `accounts.json`, `ui_auth.json`, các phiên `tdlib/<hash>/`, `khoa`, `storage_settings.json`, `appearance_settings.json`). Tuyệt đối không chạy đồng thời hai container trên cùng một thư mục phiên TDLib.
- **Đặt lại mật khẩu đăng nhập web khi quên:**
  ```bash
  docker exec -it tele-rust /usr/local/bin/tggw passwd
  ```
  Hoặc truyền qua `TG_UI_PASSWORD` cùng biến `TGGW_DATA` đang dùng (trong add-on HA là `/data/state`):
  ```bash
  docker exec -e TGGW_DATA=/data/state -e TG_UI_PASSWORD='mat-khau-moi-du-10-ky-tu' addon_local_tele_rs /usr/local/bin/tggw passwd
  ```
  Lệnh ghi nguyên tử khoá mới và thu hồi toàn bộ phiên đăng nhập web ngay lập tức.
- **Kiểm tra thành phần native trong container:**
  ```bash
  docker run --rm ghcr.io/tritue2011/tele-rust:latest version
  docker run --rm ghcr.io/tritue2011/tele-rust:latest protocol
  ```
