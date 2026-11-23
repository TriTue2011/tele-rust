# Hướng dẫn cấu hình Add-on Tele Rust

## 1. Thêm kho add-on vào Home Assistant

1. Mở **Cài đặt → Tiện ích bổ sung (Add-ons) → Cửa hàng tiện ích bổ sung**.
2. Chọn biểu tượng ba chấm góc trên bên phải → **Kho lưu trữ (Repositories)**.
3. Thêm URL kho lưu trữ: `https://github.com/TriTue2011/tele-rust`.
4. Tìm **Tele Rust**, nhấn **Cài đặt (Install)** và cấu hình các tuỳ chọn bắt buộc trước khi khởi động.

> **Lưu ý khi kho chuyển sang chế độ riêng tư (private):** Home Assistant Supervisor cần quyền kéo ảnh từ GitHub Container Registry (`ghcr.io/tritue2011/tele-rust`). Hãy tạo GitHub Personal Access Token (classic) chỉ có quyền `read:packages`, sau đó đăng nhập registry trên Home Assistant (ví dụ `ha docker registries add ghcr.io --username <github-user> --password <pat>` hoặc `docker login ghcr.io`) trước khi cài đặt hoặc cập nhật add-on.

> **Phạm vi kiến trúc & Ingress:** Add-on hiện chỉ hỗ trợ kiến trúc **`amd64`** (do thư viện tĩnh NTgCalls `3.0.7` và wheel `tdjson==1.8.69` chưa hỗ trợ `arm64`/`aarch64`). **Ingress chưa được bật** vì gateway áp dụng chính sách bảo mật `frame-ancestors 'none'`, đường dẫn gốc `/api/*` và cookie `Path=/`; hãy truy cập giao diện web trực tiếp qua cổng `3334` (nút **Mở Web UI** hoặc reverse proxy HTTPS).

## 2. Bảng tuỳ chọn cấu hình (`options`)

Mỗi tuỳ chọn trong `config.yaml` được `docker/entrypoint.sh` ánh xạ sang biến môi trường tương ứng khi khởi động:

| Tuỳ chọn (`config.yaml`) | Biến môi trường | Mặc định | Mô tả |
| --- | --- | --- | --- |
| `tg_api_id` | `TG_API_ID` | `0` (bắt buộc > 0) | Telegram API ID (`api_id`). |
| `tg_api_hash` | `TG_API_HASH` | `""` (bắt buộc) | Telegram API Hash (`api_hash`, ẩn kiểu password). |
| `tg_phone` | `TG_PHONE` | `""` | Số điện thoại tài khoản mặc định (chỉ cần nếu dùng chế độ một tài khoản cũ hoặc CLI `login`). |
| `gateway_token` | `GATEWAY_TOKEN` | `""` (bắt buộc) | Token Bearer cho Home Assistant API và thiết lập tài khoản web lần đầu. |
| `tg_ui_user` | `TG_UI_USER` | `""` | Tên đăng nhập web khởi tạo tự động ở lần chạy đầu (cần đặt cùng `tg_ui_password`). |
| `tg_ui_password` | `TG_UI_PASSWORD` | `""` | Mật khẩu khởi tạo tài khoản web lần đầu (tối thiểu 10 ký tự, không trùng tên đăng nhập). |
| `ui_cookie_secure` | `TGGW_UI_COOKIE_SECURE` | `false` (`0`) | Bật cờ `Secure` cho cookie phiên khi đặt sau reverse proxy HTTPS. |
| `https` | `GATEWAY_HTTPS` | `true` (`1`) | Gateway nhận cả `http://` lẫn `https://` trên CÙNG cổng (HTTPS dùng chứng chỉ tự ký). Cần cho mic/webcam của trình duyệt (trình duyệt chỉ cho dùng trên HTTPS hoặc localhost). `false` để tắt. |
| `tls_names` | `GATEWAY_TLS_NAMES` | `""` | Tên/IP thêm vào chứng chỉ HTTPS tự ký, cách nhau bằng dấu phẩy (ví dụ `192.0.2.10,tele.local`). Đổi giá trị thì xoá thư mục `tls/` trong dữ liệu add-on để sinh lại. |
| `rate_limit_per_minute` | `RATE_LIMIT_PER_MINUTE` | `120` | Giới hạn số yêu cầu API mỗi phút cho mỗi token/phiên (snapshot gấp 3 lần). |
| `ha_webhook_url` | `HA_WEBHOOK_URL` | `""` | URL nhận webhook sự kiện gửi về Home Assistant. |
| `ha_webhook_token` | `HA_WEBHOOK_TOKEN` | `""` | Giá trị header `X-Gateway-Token` gửi kèm yêu cầu webhook. |
| `telegram_p2p` | `TELEGRAM_P2P` | `false` (`0`) | Cho phép gọi P2P trực tiếp nếu Telegram cho phép (`1`/`true` để bật). |
| `video_profile` | `VIDEO_PROFILE` | `"standard"` | Hồ sơ khung hình video gửi đi (`standard` = `640x360@15`, `low` = `480x270@10`). |
| `telegram_rx_video` | `TELEGRAM_RX_VIDEO` | `"640x360@15"` | Độ phân giải/fps video nhận từ bên kia (`WxH@F`, 160–3840 × 120–2160, 5–60 fps). |
| `tts_voice` | `TTS_VOICE` | `"vi-VN-HoaiMyNeural"` | Giọng mặc định cho `edge-tts`. |
| `tts_rate` | `TTS_RATE` | `"+0%"` | Tốc độ đọc mặc định cho `edge-tts` (`-100%` đến `+100%`). |
| `log_level` | `LOG_LEVEL` | `"INFO"` | Mức nhật ký ra `stderr` (`DEBUG`, `INFO`, `WARNING`, `WARN`, `ERROR`, `CRITICAL`, `OFF`). |
| `telegram_debug` | `TELEGRAM_DEBUG` | `false` (`0`) | Đặt mức nhật ký mặc định thành `DEBUG` khi không chỉ định `LOG_LEVEL`. |
| `calltrace_keep` | `TELEGRAM_CALLTRACE_KEEP` | `3` | Số tệp nhật ký cuộc gọi thành công giữ lại trong `calltrace/` (`0` = tắt). |
| `calltrace_keep_bad` | `TELEGRAM_CALLTRACE_KEEP_BAD` | `30` | Số tệp nhật ký cuộc gọi lỗi giữ lại. |
| `calltrace_max_kb` | `TELEGRAM_CALLTRACE_MAX_KB` | `6144` | Dung lượng tối đa mỗi tệp nhật ký cuộc gọi tính bằng KB (`1..65536`). |
| `webrtc_log` | `TELEGRAM_WEBRTC_LOG` | `"off"` | Ghi nhật ký WebRTC từ NTgCalls vào `calltrace/webrtc.log` (`off`, `info`, `debug`). |
| `webrtc_log_mb` | `TELEGRAM_WEBRTC_LOG_MB` | `20` | Kích thước xoay vòng tối đa mỗi tệp `webrtc.log` tính bằng MB (`1..1024`). |
| `webrtc_log_loc` | `TELEGRAM_WEBRTC_LOG_LOC` | `""` | Biểu thức chính quy lọc dòng nhật ký WebRTC. |

## 3. Chạy bằng Docker độc lập (không qua Home Assistant)

Container chạy dưới người dùng không đặc quyền UID/GID `10001` (`tggw`). Nếu gắn thư mục máy chủ (bind mount) vào `/data`, cần cấp quyền sở hữu trước cho `10001:10001`:

```bash
mkdir -p ./tele-rust-data
chown 10001:10001 ./tele-rust-data
docker run -d \
  --name tele-rust \
  --restart unless-stopped \
  -p 127.0.0.1:3334:3334 \
  -v "$(pwd)/tele-rust-data:/data" \
  -e TG_API_ID=12345 \
  -e TG_API_HASH='<your-api-hash>' \
  -e GATEWAY_TOKEN='<random-64-hex-token>' \
  ghcr.io/tritue2011/tele-rust:latest
```

Ví dụ `docker-compose.yml`:

```yaml
services:
  tele-rust:
    image: ghcr.io/tritue2011/tele-rust:latest
    restart: unless-stopped
    ports:
      - "127.0.0.1:3334:3334"
    volumes:
      - tele_rust_data:/data
    environment:
      TG_API_ID: "12345"
      TG_API_HASH: "<your-api-hash>"
      GATEWAY_TOKEN: "<random-64-hex-token>"

volumes:
  tele_rust_data:
```

## 4. Lưu ý bảo mật và dữ liệu

- **Quyền dữ liệu `/data` (UID `10001`):** Khi chạy dưới Home Assistant Supervisor, `docker/entrypoint.sh` tự chuyển `TGGW_DATA` sang `/data/state` để tiến trình chỉ thiết lập quyền `0700` trên `/data/state`, giữ nguyên quyền đọc `/data/options.json`.
- **Reverse proxy HTTPS:** Bật `ui_cookie_secure: true` (`TGGW_UI_COOKIE_SECURE=1`) khi đặt sau Nginx/Caddy/Cloudflare Tunnel để trình duyệt chỉ gửi cookie phiên qua HTTPS. Không công bố cổng `3334` trực tiếp ra Internet.
- **Đặt lại mật khẩu web:** Nếu quên mật khẩu quản trị web, chạy `docker exec -it <container> /usr/local/bin/tggw passwd` (với cùng `TGGW_DATA`) để đổi mật khẩu và thu hồi toàn bộ phiên cũ.
- **Giấy phép:** Dự án theo giấy phép MIT; ảnh Docker chứa NTgCalls `3.0.7` (LGPL-3.0, kèm hướng dẫn liên kết lại tại `/licenses/NOTICE.md`) và TDLib `1.8.69` (BSL-1.0).
