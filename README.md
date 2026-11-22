# tele-rust

[![CI](https://github.com/TriTue2011/tele-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/TriTue2011/tele-rust/actions/workflows/ci.yml)
[![Image](https://github.com/TriTue2011/tele-rust/actions/workflows/image.yml/badge.svg)](https://github.com/TriTue2011/tele-rust/actions/workflows/image.yml)
[![Add-on](https://github.com/TriTue2011/tele-rust/actions/workflows/addon.yml/badge.svg)](https://github.com/TriTue2011/tele-rust/actions/workflows/addon.yml)

Gateway **Telegram viết bằng Rust** (tài khoản người dùng: nhắn tin, đa tài khoản, quản lý bộ nhớ đệm và gọi thoại/video P2P) cho **Home Assistant** và giao diện web nhúng.

> **Tuyên bố miễn trừ trách nhiệm:** Đây là **dự án không chính thức, không liên kết, không được tài trợ hay xác nhận bởi Telegram**. Phần mềm tương tác với mạng Telegram thông qua TDLib (`libtdjson` 1.8.69, nạp động lúc chạy) và NTgCalls (`3.0.7`, liên kết tĩnh). Người dùng tự chịu trách nhiệm khi triển khai và sử dụng với tài khoản của mình.

- **Phiên bản hiện tại:** `v0.7.1` (`Cargo.toml` và `addon/tele-rs/config.yaml`)
- **Ảnh Docker (`linux/amd64`):** `ghcr.io/tritue2011/tele-rust:latest` (cùng thẻ `:v0.7.1`, `:0.7.1`, `:sha-<commit>`)
- **Kiến trúc:** `src/contract.rs` định nghĩa hợp đồng `TdLink`, `CallControl`, `ChatControl` tách biệt tầng TDLib (`src/td/`), cuộc gọi WebRTC/NTgCalls (`src/calls/`), dịch vụ chat/danh bạ/lưu trữ (`src/chat/`, `src/storage.rs`), quản lý đa tài khoản (`src/account_router.rs`), xác thực web (`src/ui_auth.rs`) và máy chủ Axum REST (`src/rest.rs`).

## Tính năng thật của v0.7.1

### 1. Xác thực web và quản lý đa tài khoản
- **Đăng nhập web bằng tên + mật khẩu:** Thiết lập lần đầu bằng token gateway một lần (`POST /ui/api/auth/setup`) hoặc khởi tạo tự động qua `TG_UI_USER` + `TG_UI_PASSWORD`; hỗ trợ «Ghi nhớ trên trình duyệt này» tối đa 90 ngày, đổi mật khẩu tại giao diện hoặc khôi phục qua CLI `tggw passwd`, đăng xuất phiên hiện tại và đăng xuất mọi nơi.
- **Đa tài khoản Telegram (`/ui/index.html`):** Thêm/quản lý nhiều tài khoản đồng thời bằng số điện thoại + OTP/2FA hoặc quét mã QR (thư viện `web/qrcode.min.js` nhúng nội bộ). Khoá phiên 15 phút sau 3 lần nhập sai mã/mật khẩu, tôn trọng `FLOOD_WAIT` (`retry_after_seconds`), tự đóng client bỏ dở sau 10 phút. Khi xoá tài khoản (`DELETE /ui/api/accounts/:login_id`), dữ liệu phiên được chuyển sang `tdlib/.da-xoa-*` để tránh mất dữ liệu ngoài ý muốn. Chọn tài khoản qua `?account=<phone>` hoặc trường `account` trong JSON.

### 2. Giao diện trò chuyện, soạn thảo và cài đặt
- **Danh sách hội thoại kiểu macOS:** Sắp xếp và rút gọn bộ đếm chưa đọc chính xác (không cắt `999+`), huy hiệu tắt tiếng, xác minh, trạng thái trực tuyến, biểu tượng tự xoá, dấu tick đã đọc (`✓` / `✓✓` đồng bộ tức thì qua sự kiện `chat_read_outbox`), ảnh thu nhỏ và nhãn cuộc gọi nhỡ/đi/đến/từ chối.
- **Tin hệ thống tiếng Việt:** Định dạng rõ ràng các thông báo thêm/rời thành viên, ghim tin, đổi ảnh/tiêu đề, thay đổi thời gian tự xoá và lịch sử cuộc gọi.
- **Ô soạn thảo định dạng WYSIWYG:** Hỗ trợ thanh công cụ định dạng chữ đậm (`B`), nghiêng (`I`), gạch chân (`U`), gạch giữa (`S`), mã nguồn (`</>`), xoá định dạng (`Tx`) và gửi `entities` chuẩn UTF-16 (`bold`, `italic`, `underline`, `strikethrough`, `code`, `pre`, `spoiler`, `blockquote`), chặn gửi dở khi gõ tiếng Việt IME và chống XSS khi dán nội dung.
- **Ngăn Thông tin hội thoại:** Xem chi tiết cá nhân/nhóm/kênh, bật/tắt thông báo riêng, cấu hình tự xoá tin đồng bộ hai chiều với Telegram (tắt hoặc 1–365 ngày tròn ngày, bắt buộc xác nhận), ghim/lưu trữ, chặn/bỏ chặn, tìm kiếm tin nhắn trong hội thoại, xem/tìm thành viên nhóm, hiển thị liên kết mời cho quản trị viên, xoá lịch sử/xoá hội thoại/rời nhóm (có hộp thoại xác nhận `confirm: true`).
- **Danh bạ:** Sắp xếp theo chữ cái (`A–Z`, `#` qua ICU collator tiếng Việt) hoặc lần truy cập gần nhất, danh sách bị chặn, thêm/sửa/xoá liên hệ, mở chat riêng, chặn kèm tuỳ chọn báo spam/xoá hội thoại.
- **Cài đặt:** Điều hướng 2 bước gồm Hồ sơ (sửa tên, họ, tiểu sử tối đa 70 ký tự, username, bắt buộc `confirm: true`), Giao diện (10 chủ đề màu, 4 họ phông, cỡ chữ 12–20 px, kiểu bong bóng, màu nhấn, giãn dòng, nền khung chat, chủ đề khối mã và popover chỉnh nhanh «Aa» đồng bộ 2 chiều) và Dữ liệu và lưu trữ.
- **Dữ liệu và lưu trữ:** Hai chế độ `local` (lưu cục bộ kèm quy tắc tự động tải theo loại chat/loại tệp) và `stream` (xem trực tuyến với `stream_ttl_s`), dọn tự động định kỳ mỗi 600 giây theo hạn mức, hỗ trợ ngoại lệ từng hội thoại đặc biệt là mức **«Mãi mãi»** (cả dọn tự động lẫn xoá thủ công `all`/`type` đều bỏ qua hội thoại giữ mãi mãi, ngoại trừ xoá riêng chính hội thoại đó), cảnh báo khi đồng thời bật tự xoá tin và giữ mãi mãi, và bắt buộc `confirm: true` khi siết chặt chính sách lưu trữ hoặc xoá bộ nhớ đệm.

### 3. Cuộc gọi thoại/video P2P và tích hợp Home Assistant
- **Gọi thoại/video qua NTgCalls `3.0.7`:** Tạo cuộc gọi đi (`/api/live/call`), nhận (`/api/live/accept`), cúp máy (`/api/live/hangup`), đổi nguồn mic/camera trong cuộc gọi (`/api/live/media`), phát âm thanh/TTS (`/api/live/say` qua `edge-tts` + `ffmpeg`), xem khung hình video MJPEG (`/api/live/view/:call_id`) và ảnh chụp JPEG (`/api/live/snapshot/:call_id`).
- **Âm thanh trình duyệt và thiết bị HA:** Hỗ trợ `mic: "browser"` (nhận PCM s16le 16 kHz mono qua `POST /api/live/audio/in/:call_id`), `speaker: "browser"` (cấp token media 192 bit ngắn hạn 600 giây qua `POST /api/live/media` → `GET /api/live/audio/out/:call_id?t=...` luồng WAV 48 kHz mono), đồng bộ danh mục thiết bị Home Assistant (`/api/live/devices`, `/api/live/ha_devices`), nguồn camera RTSP/HTTP/`black` và cửa bộ đàm (`r1_speaker`/`dahua_talk`).
- **Sự kiện và Webhook:** Hàng đợi sự kiện có con trỏ (`/api/events?since=`), long-poll giao diện (`/api/feed?since=&wait=`) và đẩy webhook sang Home Assistant (`HA_WEBHOOK_URL` kèm lọc DNS chống SSRF link-local).

## Chưa làm / Giới hạn đã biết

- **Các mục Cài đặt đánh dấu «Sắp có»:** *Thông báo và âm thanh*, *Quyền riêng tư và bảo mật*, *Thư mục hội thoại* và *Thiết bị* (phiên đăng nhập Telegram) chỉ mới có mục điều hướng giữ chỗ, chưa có cấu hình chức năng.
- **Camera trình duyệt:** Cấu hình chấp nhận `camera: "browser"`, nhưng bật video với nguồn này trả `501 BROWSER_VIDEO_UNAVAILABLE` (chưa có `/api/live/video/in/:call_id`). Hãy dùng `black` hoặc URL RTSP/HTTP.
- **macOS: chưa hỗ trợ:** Chưa cung cấp workflow dựng hay nhị phân phát hành cho macOS vì tổ hợp TDLib `1.8.69` và NTgCalls `3.0.7` trên macOS chưa được kiểm chứng thực tế trên máy đích.
- **Kiến trúc `arm64`/`aarch64`:** Ảnh Docker và add-on hiện chỉ dựng cho `linux/amd64` do nhị phân tĩnh NTgCalls `3.0.7` và wheel `tdjson==1.8.69` trên PyPI chỉ có gói `linux/amd64`.
- **Home Assistant Ingress:** Chưa bật Ingress trên add-on vì gateway đặt header `Content-Security-Policy: ... frame-ancestors 'none'`, dùng đường dẫn gốc tuyệt đối `/api/*` và cookie `Path=/`. Truy cập Web UI qua cổng `3334`.
- **Loa Google Cast đã bỏ khỏi luồng khuyến nghị:** Giao diện và tài liệu không còn khuyến nghị thiết lập loa Cast mới (ưu tiên loa trình duyệt `browser` hoặc cửa bộ đàm nội bộ); route `/api/live/cast/:call_id/:file` và bộ giải mã `cast://` trong `src/calls/cast.rs` chỉ giữ lại để tương thích ngược mã hiện có.
- **Các đường REST cũ (`/api/calls`, `/api/call`, `/api/answer_call`, `/api/hangup_call`, `/api/audio/in`, `/api/audio/upload`):** Được đăng ký ở tầng router `src/rest.rs` nhưng `ChatService` trả `404 NOT_FOUND`; mọi điều khiển cuộc gọi dùng nhóm `/api/live/*`.
- **Chế độ giả lập (`GATEWAY_MOCK=1`):** Bản Rust không hỗ trợ giả lập (luôn từ chối khởi động với mã thoát `2`).

## Cách chạy

### 1. Chạy bằng Docker (`docker run`)

Container chạy dưới người dùng không root UID/GID `10001` (`tggw`), volume `/data`, cổng mặc định `3334`:

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

### 2. Chạy bằng Docker Compose

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
      TG_API_HASH: "fake-api-hash-placeholder"
      GATEWAY_TOKEN: "fake-gateway-token-placeholder"
      TGGW_UI_COOKIE_SECURE: "0"
    healthcheck:
      test: ["CMD-SHELL", "curl -fsS http://127.0.0.1:3334/health >/dev/null || exit 1"]
      interval: 30s
      timeout: 5s
      retries: 3

volumes:
  tele_rust_data:
```

### 3. Cài đặt Add-on Home Assistant

1. Trong Home Assistant, mở **Cài đặt → Tiện ích bổ sung → Cửa hàng tiện ích bổ sung → ba chấm góc trên → Kho lưu trữ (Repositories)**.
2. Thêm URL: `https://github.com/TriTue2011/tele-rust`.
3. Cài đặt **Tele Rust**, điền `tg_api_id`, `tg_api_hash`, `gateway_token` trong tab **Cấu hình**, rồi khởi động add-on.
4. Nếu kho được chuyển sang chế độ riêng tư (private), cấu hình đăng nhập `ghcr.io` trên Home Assistant bằng classic PAT có quyền `read:packages` trước khi cài/cập nhật (chi tiết xem [addon/tele-rs/DOCS.md](addon/tele-rs/DOCS.md) và [docs/TRIEN-KHAI.md](docs/TRIEN-KHAI.md)).

### 4. Tự dựng từ mã nguồn và các lệnh CLI

- **Dựng ảnh Docker tự đủ:**
  ```bash
  docker build --build-arg APP_VERSION=0.7.1 -t tele-rust:local .
  # Tắt cài edge-tts (giảm dung lượng nếu không cần TTS):
  docker build --build-arg WITH_TTS=0 -t tele-rust:no-tts .
  ```
- **Dựng trực tiếp bằng Cargo (Ubuntu/Debian `amd64`, Rust 1.90 + `cmake`, `make`, `pkg-config`, `g++`, `zlib1g-dev`):**
  ```bash
  cargo build --release --bin tggw
  TDJSON_LIB=/duong/dan/libtdjson.so ./target/release/tggw version
  ```
- **Dựng qua máy dựng từ xa (tùy chọn):** cấu hình `TGGW_BUILD_HOST` hoặc tệp `.tggw-build-host` rồi chạy `./build.sh build` / `./build.sh test`.

| Lệnh CLI (`tggw`) | Mục đích |
| --- | --- |
| `tggw serve` | Khởi chạy máy chủ HTTP REST + giao diện web tại `GATEWAY_HOST:REST_PORT`. |
| `tggw version` | Nạp `TDJSON_LIB` và in phiên bản TDLib (`TDLib 1.8.69`). |
| `tggw protocol` | Kiểm tra liên kết tĩnh NTgCalls và danh sách phiên bản `tgcalls` hỗ trợ. |
| `tggw login` | Đăng nhập một tài khoản qua terminal hoặc `<TGGW_DATA>/login_code.txt`, `login_password.txt`. |
| `tggw passwd` | Đặt lại mật khẩu đăng nhập web trên máy chủ và thu hồi toàn bộ phiên cũ. |

## Biến môi trường

Bảng dưới đây liệt kê toàn bộ biến môi trường thời gian chạy được đọc trong mã nguồn Rust (`src/`):

| Biến | Bí danh Python | Mặc định | Mô tả |
| --- | --- | --- | --- |
| `GATEWAY_TOKEN` | — | *(bắt buộc)* | Token Bearer cho API máy-gọi-máy và token thiết lập tài khoản web lần đầu. |
| `TG_API_ID` | `TELEGRAM_API_ID` | *(bắt buộc)* | Telegram API ID (số nguyên dương `i32`). |
| `TG_API_HASH` | `TELEGRAM_API_HASH` | *(bắt buộc)* | Telegram API Hash (`String`, luôn bị che trong log/trace). |
| `TG_PHONE` | — | `""` | Số điện thoại tài khoản mặc định cho chế độ một tài khoản cũ hoặc `tggw login`. |
| `TGGW_DATA` | `DATA_DIR` | `/data` | Thư mục lưu dữ liệu phiên TDLib, `accounts.json`, `ui_auth.json`, sự kiện và nhật ký. |
| `TDJSON_LIB` | — | `libtdjson.so` | Đường dẫn tới thư viện dùng chung `libtdjson.so` (`1.8.69`). |
| `GATEWAY_HOST` | — | `0.0.0.0` | Địa chỉ IP mà máy chủ HTTP lắng nghe. |
| `REST_PORT` | — | `3334` | Cổng TCP mà máy chủ HTTP lắng nghe. |
| `RATE_LIMIT_PER_MINUTE` | — | `120` | Hạn mức số yêu cầu API mỗi phút (cửa sổ trượt 60 giây; snapshot gấp 3 lần). |
| `TG_UI_USER` | — | `""` | Tên đăng nhập web khởi tạo tự động khi chưa có `ui_auth.json` (phải đặt cùng `TG_UI_PASSWORD`). |
| `TG_UI_PASSWORD` | — | `""` | Mật khẩu web khởi tạo tự động hoặc mật khẩu mới truyền không tương tác cho `tggw passwd`. |
| `TGGW_UI_COOKIE_SECURE` | — | `0` | Đặt `1`/`true`/`yes`/`on` để ép gắn cờ `Secure` cho cookie phiên `tggw_session` sau proxy HTTPS. |
| `HA_WEBHOOK_URL` | — | `""` | URL HTTP/HTTPS nhận webhook sự kiện gửi về Home Assistant (để trống để tắt). |
| `HA_WEBHOOK_TOKEN` | — | `""` | Giá trị bí mật gửi trong header `X-Gateway-Token` khi gọi webhook. |
| `TELEGRAM_P2P` | — | `0` | Đặt `1`/`true`/`yes`/`on` để cho phép kết nối P2P trực tiếp nếu Telegram cho phép. |
| `VIDEO_PROFILE` | — | `standard` | Độ phân giải video gửi đi (`low` → `480x270@10`; khác → `640x360@15`). |
| `TELEGRAM_RX_VIDEO` | — | `640x360@15` | Độ phân giải/fps video nhận dạng `WxH@F` (`160–3840 × 120–2160`, `5–60` fps). |
| `TTS_VOICE` | — | `vi-VN-HoaiMyNeural` | Tên giọng mặc định cho `edge-tts` (`1–128` ký tự chữ/số/`-`). |
| `TTS_RATE` | — | `+0%` | Tốc độ đọc mặc định cho `edge-tts` (`+N%` hoặc `-N%`, từ `-100%` đến `+100%`). |
| `LOG_LEVEL` | — | `INFO` (hoặc `DEBUG` nếu `TELEGRAM_DEBUG=1`) | Mức ghi log ra `stderr`: `DEBUG`, `INFO`, `WARNING`, `WARN`, `ERROR`, `CRITICAL`, `OFF`. |
| `TELEGRAM_DEBUG` | — | `0` | Đặt `1`/`true`/`yes`/`on` để nâng mức log mặc định lên `DEBUG` nếu không đặt `LOG_LEVEL`. |
| `TELEGRAM_CALLTRACE_KEEP` | — | `3` | Số tệp nhật ký cuộc gọi bình thường tối đa giữ lại (`0..10000`; `0` = tắt calltrace). |
| `TELEGRAM_CALLTRACE_KEEP_BAD` | — | `30` | Số tệp nhật ký cuộc gọi có đánh dấu lỗi giữ lại (`0..10000`). |
| `TELEGRAM_CALLTRACE_MAX_KB` | — | `6144` | Dung lượng tối đa mỗi tệp `calltrace/<id>.log` tính bằng KB (`1..65536`). |
| `TELEGRAM_WEBRTC_LOG` | — | `""` | Mức log WebRTC của NTgCalls ghi ra `calltrace/webrtc.log`: trống (tắt), `info`, `debug`. |
| `TELEGRAM_WEBRTC_LOG_MB` | — | `20` | Dung lượng tối đa mỗi tệp xoay vòng `webrtc.log` tính bằng MB (`1..1024`). |
| `TELEGRAM_WEBRTC_LOG_LOC` | — | `""` | Biểu thức chính quy lọc dòng nhật ký WebRTC theo nội dung đã che bí mật. |
| `GATEWAY_MOCK` | — | `0` | Chỉ chấp nhận `0`; đặt `1` sẽ khiến tiến trình từ chối khởi động ngay ở `config::validate`. |

## Bảng route REST

Mọi đường dẫn có xác thực đều chấp nhận `Authorization: Bearer <GATEWAY_TOKEN>` hoặc cookie phiên `tggw_session` (các yêu cầu ghi bằng cookie bắt buộc vượt qua kiểm tra CSRF bằng `X-Requested-With: fetch` hoặc khớp `Origin` với `Host`). Đường dẫn tĩnh chỉ trả 200 mà không cần Bearer; xem chi tiết payload và mã lỗi tại [docs/API.md](docs/API.md).

| Phương thức | Đường dẫn | Xác thực | Tham số chính & confirm | Mô tả |
| --- | --- | --- | --- | --- |
| `GET` | `/health` | Không | — | Trả trạng thái (`ok`/`degraded`), `version`, `uptime_seconds`, số tài khoản online. |
| `GET` | `/` | Không | — | Phục vụ trang giao diện trò chuyện nhúng (`web/app.html`). |
| `GET` | `/app.html` | Không | — | Bí danh phục vụ `web/app.html`. |
| `GET` | `/ui/app.html` | Không | — | Đường dẫn chuẩn tới giao diện trò chuyện (`web/app.html`). |
| `GET` | `/index.html` | Không | — | Phục vụ trang đăng nhập web & quản lý tài khoản (`web/index.html`). |
| `GET` | `/ui/` | Không | — | Bí danh phục vụ `web/index.html`. |
| `GET` | `/ui/index.html` | Không | — | Đường dẫn chuẩn tới trang quản lý tài khoản (`web/index.html`). |
| `GET` | `/ui/qrcode.min.js` | Không | — | Phục vụ thư viện tạo mã QR nội bộ (`qrcode-generator` 1.4.4). |
| `GET` | `/ui/api/auth/status` | Công khai (kiểm cookie nếu có) | Cookie `tggw_session` | Kiểm tra đã thiết lập tài khoản web chưa và trạng thái đăng nhập. |
| `POST` | `/ui/api/auth/setup` | Token gateway một lần | `token`, `ten`, `mat_khau` (≥10 ký tự) | Tạo tài khoản web lần đầu và cấp cookie phiên `HttpOnly`. |
| `POST` | `/ui/api/auth/login` | Tên + mật khẩu | `ten`, `mat_khau`, `ghi_nho` | Đăng nhập giao diện web (chống brute-force 5 lần / 15 phút). |
| `POST` | `/ui/api/auth/logout` | Cookie + CSRF | Cookie `tggw_session` | Thu hồi phiên hiện tại và xoá cookie trình duyệt. |
| `POST` | `/ui/api/auth/logout-all` | Cookie + CSRF | Cookie `tggw_session` | Thu hồi toàn bộ phiên đăng nhập web trên mọi thiết bị. |
| `POST` | `/ui/api/auth/password` | Cookie + CSRF | `cu`, `moi` (≥10 ký tự) | Đổi mật khẩu web; giữ phiên hiện tại và thu hồi các phiên khác. |
| `POST` | `/ui/api/accounts` | Bearer hoặc Cookie+CSRF | `phone` | Bắt đầu quy trình đăng nhập tài khoản Telegram bằng mã OTP. |
| `POST` | `/ui/api/accounts/qr` | Bearer hoặc Cookie+CSRF | `phone` | Bắt đầu phiên đăng nhập tài khoản Telegram bằng mã QR. |
| `GET` | `/ui/api/accounts/:login_id` | Bearer hoặc Cookie | Tham số đường dẫn `:login_id` | Lấy trạng thái đăng nhập hiện tại (`code`/`qr`/`password`/`done`) và `qr_link`. |
| `DELETE` | `/ui/api/accounts/:login_id` | Bearer hoặc Cookie+CSRF | `:login_id` (số điện thoại) | Đóng phiên TDLib an toàn và lưu trữ dữ liệu vào `tdlib/.da-xoa-*`. |
| `POST` | `/ui/api/accounts/:login_id/code` | Bearer hoặc Cookie+CSRF | `code` (5–6 chữ số) | Gửi mã xác thực OTP Telegram (khoá 15 phút sau 3 lần sai). |
| `POST` | `/ui/api/accounts/:login_id/password` | Bearer hoặc Cookie+CSRF | `password` | Gửi mật khẩu 2FA Telegram cho phiên đang chờ mật khẩu. |
| `POST` | `/ui/api/accounts/:login_id/qr-confirm` | Bearer hoặc Cookie+CSRF | — | Xác nhận thủ công hoàn tất phiên QR khi TDLib đã `Ready`. |
| `GET` | `/api/accounts` | Bearer hoặc Cookie | — | Liệt kê danh sách tài khoản trong sổ và trạng thái `online`/`offline`. |
| `GET` | `/api/me` | Bearer hoặc Cookie | `?account=` | Lấy thông tin ngắn gọn (`id`, `name`, `phone`) của tài khoản đang chọn. |
| `GET` | `/api/profile` | Bearer hoặc Cookie | `?account=` (bắt buộc nếu nhiều TK) | Lấy hồ sơ đầy đủ (`first_name`, `last_name`, `bio`, `username`, `phone_masked`). |
| `POST` | `/api/profile` | Bearer hoặc Cookie+CSRF | `confirm: true`, `first_name`, `last_name`, `bio`, `username`, `?account=` | Cập nhật hồ sơ Telegram (bắt buộc `confirm: true`). |
| `GET` | `/api/chats` | Bearer hoặc Cookie | `?limit=1..200`, `?account=` | Lấy danh sách hội thoại chính kèm số chưa đọc, tự xoá, trạng thái. |
| `GET` | `/api/chats/:chat_id/messages` | Bearer hoặc Cookie | `:chat_id`, `?before=`, `?limit=1..100`, `?q=`, `?account=` | Lấy lịch sử tin nhắn hoặc tìm kiếm tin nhắn trong hội thoại khi có `q`. |
| `GET` | `/api/chats/:chat_id/info` | Bearer hoặc Cookie | `:chat_id`, `?account=` | Lấy thông tin chi tiết hội thoại (cá nhân/nhóm/kênh), quyền và thông báo. |
| `GET` | `/api/chats/:chat_id/members` | Bearer hoặc Cookie | `:chat_id`, `?q=`, `?limit=1..50`, `?account=` | Liệt kê và tìm kiếm thành viên nhóm/siêu nhóm. |
| `POST` | `/api/chats/:chat_id/settings` | Bearer hoặc Cookie+CSRF | `:chat_id`, `notification`, `auto_delete` (`seconds`, `confirm: true`), `pin`, `archive`, `mark_unread` | Cập nhật thông báo, ghim, lưu trữ, chưa đọc và tự xoá (đổi `auto_delete` cần `confirm: true`). |
| `POST` | `/api/chats/:chat_id/clear_history` | Bearer hoặc Cookie+CSRF | `:chat_id`, `confirm: true`, `revoke`, `remove_from_list` | Xoá toàn bộ lịch sử hội thoại (bắt buộc `confirm: true`). |
| `POST` | `/api/chats/:chat_id/delete` | Bearer hoặc Cookie+CSRF | `:chat_id`, `confirm: true`, `revoke` | Xoá hội thoại riêng hoặc rời nhóm/kênh (bắt buộc `confirm: true`). |
| `POST` | `/api/chats/:chat_id/leave` | Bearer hoặc Cookie+CSRF | `:chat_id`, `confirm: true` | Rời khỏi nhóm hoặc kênh (bắt buộc `confirm: true`). |
| `POST` | `/api/chats/:chat_id/send` | Bearer hoặc Cookie+CSRF | `:chat_id`, `text`, `reply_to`, `parse_mode`, `entities` | Gửi tin nhắn văn bản hoặc văn bản có định dạng UTF-16 `entities`. |
| `POST` | `/api/chats/:chat_id/send_file` | Bearer hoặc Cookie+CSRF | `:chat_id`, `?kind=`, `?name=`, `?caption=`, `?parse_mode=`, `?reply_to=`, thân thô ≤64 MB | Gửi tệp ảnh, video, tin thoại (tự đổi Ogg/Opus), âm thanh hoặc tài liệu. |
| `POST` | `/api/chats/:chat_id/send_album` | Bearer hoặc Cookie+CSRF | `:chat_id`, `items` (2–10 mục), `reply_to`, `parse_mode` | Gửi album nhiều ảnh/video/tệp/âm thanh (tổng ≤64 MB). |
| `POST` | `/api/chats/:chat_id/read` | Bearer hoặc Cookie+CSRF | `:chat_id`, `message_ids` | Đánh dấu đã đọc tối đa 100 tin nhắn trong hội thoại. |
| `GET` | `/api/contacts` | Bearer hoặc Cookie | `?sort=last_seen\|name`, `?account=` | Lấy danh bạ kèm trạng thái hoạt động và phân nhóm chữ cái khi `sort=name`. |
| `GET` | `/api/contacts/blocked` | Bearer hoặc Cookie | `?account=` | Lấy danh sách người dùng đang bị chặn. |
| `POST` | `/api/contacts/open` | Bearer hoặc Cookie+CSRF | `user_id`, `?account=` | Mở hoặc tạo hội thoại riêng với `user_id` và trả `chat_id`. |
| `POST` | `/api/contacts/add` | Bearer hoặc Cookie+CSRF | `phone`, `first_name`, `last_name`, `share_my_phone` | Nhập số điện thoại mới vào danh bạ Telegram. |
| `POST` | `/api/contacts/edit` | Bearer hoặc Cookie+CSRF | `user_id`, `first_name`, `last_name` | Sửa tên liên hệ đã có trong danh bạ. |
| `POST` | `/api/contacts/remove` | Bearer hoặc Cookie+CSRF | `user_ids` (1–100 phần tử) | Xoá liên hệ khỏi danh bạ Telegram. |
| `POST` | `/api/contacts/block` | Bearer hoặc Cookie+CSRF | `user_id`, `report_spam`, `delete_chat` | Chặn người dùng (tuỳ chọn báo spam và xoá hội thoại). |
| `POST` | `/api/contacts/unblock` | Bearer hoặc Cookie+CSRF | `user_id` | Bỏ chặn người dùng. |
| `GET` | `/api/search` | Bearer hoặc Cookie | `?q=`, `?account=` | Tìm hội thoại trên danh sách chat của tài khoản. |
| `GET` | `/api/feed` | Bearer hoặc Cookie | `?since=`, `?wait=0..25` | Long-poll luồng cập nhật tin nhắn, đã đọc và cuộc gọi cho giao diện web. |
| `GET` | `/api/events` | Bearer hoặc Cookie | `?since=` | Lấy các sự kiện tuần tự từ spool (`account_status`, `call_*`, tin mới) cho HA. |
| `GET` | `/api/file/:file_id` | Bearer hoặc Cookie | `:file_id`, `?account=` | Tải tệp/ảnh/âm thanh từ TDLib với tiêu đề cô lập XSS và giới hạn 64 MB. |
| `GET` | `/api/calltrace` | Bearer hoặc Cookie | `?account=` | Liệt kê các tệp nhật ký cuộc gọi `calltrace/` của tài khoản. |
| `GET` | `/api/calltrace/:call_id` | Bearer hoặc Cookie | `:call_id`, `?account=` | Tải tệp nhật ký cuộc gọi dưới dạng `attachment` đã lọc bí mật. |
| `GET` | `/api/storage/usage` | Bearer hoặc Cookie | `?account=` (bắt buộc nếu nhiều TK) | Thống kê dung lượng bộ nhớ đệm TDLib theo loại tệp và top 20 hội thoại. |
| `GET` | `/api/storage/janitor` | Bearer hoặc Cookie | `?account=` (bắt buộc nếu nhiều TK) | Lấy báo cáo lần dọn bộ nhớ đệm gần nhất (`freed_bytes`, `remaining_bytes`, `error`). |
| `GET` | `/api/settings/storage` | Bearer hoặc Cookie | `?account=` (bắt buộc nếu nhiều TK) | Đọc chính sách lưu trữ (`mode`, `keep`, `exceptions`, `auto_download`, `max_cache_bytes`). |
| `POST` | `/api/settings/storage` | Bearer hoặc Cookie+CSRF | Cấu hình lưu trữ đầy đủ, `confirm: true` khi siết chặt hoặc sang `stream` | Lưu chính sách lưu trữ; yêu cầu `confirm: true` khi giảm hạn giữ/dung lượng hoặc sang `stream`. |
| `POST` | `/api/storage/clear` | Bearer hoặc Cookie+CSRF | `confirm: true`, `scope` (`all`\|`chat`\|`type`), `chat_id`, `file_types` | Xoá bộ nhớ đệm thủ công (bắt buộc `confirm: true`; `all`/`type` bỏ qua ngoại lệ Mãi mãi). |
| `GET` | `/api/settings/appearance` | Bearer hoặc Cookie | `?account=` (bắt buộc nếu nhiều TK) | Đọc cấu hình giao diện trò chuyện (`theme`, `font_family`, `font_size`, `bubble_style`, `accent`, ...). |
| `POST` | `/api/settings/appearance` | Bearer hoặc Cookie+CSRF | `font_family`, `font_size`, `theme`, `bubble_style`, `accent`, ... | Kiểm hợp lệ và lưu cấu hình giao diện theo phiên tài khoản. |
| `POST` | `/api/messages/edit` | Bearer hoặc Cookie+CSRF | `chat_id`, `message_id`, `text`, `parse_mode`, `entities` | Chỉnh sửa nội dung tin nhắn đã gửi. |
| `POST` | `/api/messages/delete` | Bearer hoặc Cookie+CSRF | `chat_id`, `message_ids`, `revoke` | Xoá tin nhắn trong hội thoại. |
| `POST` | `/api/messages/forward` | Bearer hoặc Cookie+CSRF | `to_chat_id`, `from_chat_id`, `message_ids` | Chuyển tiếp tin nhắn sang hội thoại khác. |
| `POST` | `/api/messages/react` | Bearer hoặc Cookie+CSRF | `chat_id`, `message_id`, `emoji`, `on` | Thêm hoặc gỡ biểu cảm phản hồi cho tin nhắn. |
| `POST` | `/api/resolve` | Bearer hoặc Cookie+CSRF | `peer` (`@username`, `+phone`, hoặc ID số) | Phân giải người nhận thành `chat_id`. |
| `POST` | `/api/translate` | Bearer hoặc Cookie+CSRF | `text`, `to_language_code` | Dịch văn bản qua dịch vụ dịch tích hợp của Telegram/TDLib. |
| `POST` | `/api/do/:name` | Bearer hoặc Cookie+CSRF | `:name` (30 dịch vụ), tham số theo tên (vd `confirm: true` cho `delete_chat_history`) | Thực thi các dịch vụ quản trị chat/poll/nhóm tương thích tích hợp HA. |
| `POST` | `/api/send_message` | Bearer hoặc Cookie+CSRF | `peer`, `message`, `account` | Gửi tin nhắn văn bản nhanh qua đường tương thích bản Python. |
| `POST` | `/api/send_media` | Bearer hoặc Cookie+CSRF | `peer`, `media_b64`, `filename`, `caption`, `account` | Gửi tài liệu base64 qua đường tương thích bản Python. |
| `POST` | `/api/send_voice` | Bearer hoặc Cookie+CSRF | `peer`, `audio_b64` hoặc `tts_text`, `tts_voice`, `tts_rate`, `account` | Gửi tin thoại từ âm thanh base64 hoặc `edge-tts` (đổi sang Ogg/Opus). |
| `GET` | `/api/calls` | Bearer hoặc Cookie | — | Đường cũ trong router; `ChatService` trả `404` (dùng `GET /api/live/calls`). |
| `POST` | `/api/call` | Bearer hoặc Cookie+CSRF | — | Đường cũ trong router; `ChatService` trả `404` (dùng `POST /api/live/call`). |
| `POST` | `/api/answer_call` | Bearer hoặc Cookie+CSRF | — | Đường cũ trong router; `ChatService` trả `404` (dùng `POST /api/live/accept`). |
| `POST` | `/api/hangup_call` | Bearer hoặc Cookie+CSRF | — | Đường cũ trong router; `ChatService` trả `404` (dùng `POST /api/live/hangup`). |
| `POST` | `/api/audio/in` | Bearer hoặc Cookie+CSRF | — | Đường cũ trong router; `ChatService` trả `404` (dùng `POST /api/live/audio/in/:call_id`). |
| `POST` | `/api/audio/upload` | Bearer hoặc Cookie+CSRF | — | Đường cũ trong router; `ChatService` trả `404`. |
| `GET` | `/api/live/calls` | Bearer hoặc Cookie | `?account=` | Lấy tối đa 10 cuộc gọi gần nhất/đang chạy của tài khoản hoặc toàn gateway. |
| `POST` | `/api/live/call` | Bearer hoặc Cookie+CSRF | `peer` hoặc `user_id`, `video`, `mic`, `camera`, `speaker`, `playback`, `account` | Bắt đầu cuộc gọi đi thoại hoặc video qua NTgCalls. |
| `POST` | `/api/live/accept` | Bearer hoặc Cookie+CSRF | `id`, `account` | Nghe cuộc gọi đến đang ở trạng thái `ringing`. |
| `POST` | `/api/live/hangup` | Bearer hoặc Cookie+CSRF | `id`, `account` | Kết thúc cuộc gọi đang diễn ra hoặc đang đổ chuông. |
| `GET` | `/api/live/settings` | Bearer hoặc Cookie | `?account=` | Đọc thiết bị mặc định cho cuộc gọi (`mic`, `camera`, `speaker`, `playback`, `auto`; che bí mật). |
| `POST` | `/api/live/settings` | Bearer hoặc Cookie+CSRF | `mic`, `mic_format`, `camera`, `speaker`, `playback`, `account` | Lưu thiết bị mặc định cho các cuộc gọi tiếp theo vào `live_settings.json`. |
| `GET` | `/api/live/devices` | Bearer hoặc Cookie | `?account=` | Trả `{settings, auto, catalog}` gồm thiết bị mặc định và danh mục thiết bị HA. |
| `POST` | `/api/live/ha_devices` | Bearer hoặc Cookie+CSRF | `mic`, `speaker`, `camera` (`[{id, name}]`, tối đa 64 mục/nhóm), `account` | Cập nhật và lọc danh mục thiết bị từ Home Assistant vào `ha_devices.json`. |
| `POST` | `/api/live/media` | Bearer hoặc Cookie+CSRF | `id`, `action: "audio_out"` hoặc `mic`/`mic_muted`/`camera`/`camera_off`/`video` | Cấp URL token âm thanh trình duyệt hoặc đổi nguồn mic/camera khi đang gọi. |
| `POST` | `/api/live/playback` | Bearer hoặc Cookie+CSRF | `id`, `kind` (`file`\|`external`\|`shell`\|`push`), `url`/`cmd` | Đổi kênh phát âm thanh nhận từ bên kia trong lúc đang gọi. |
| `POST` | `/api/live/say` | Bearer hoặc Cookie+CSRF | `id`, `text` (1–1000 ký tự) hoặc `audio_b64`, `voice`, `rate` | Phát giọng nói TTS hoặc đoạn âm thanh vào mic của cuộc gọi đang hoạt động. |
| `POST` | `/api/live/audio/in/:call_id` | Bearer hoặc Cookie+CSRF | `:call_id`, thân thô PCM s16le 16 kHz mono ≤64 KiB | Đẩy khung âm thanh mic từ trình duyệt vào nguồn loopback của cuộc gọi. |
| `GET` | `/api/live/audio/out/:call_id` | Token âm thanh `?t=` (192 bit, hạn 600s) | `:call_id`, `?t=` | Phát luồng WAV PCM 48 kHz mono tiếng từ bên kia (tối đa 2 người nghe). |
| `GET` | `/api/live/view/:call_id` | Bearer hoặc Cookie | `:call_id` | Luồng MJPEG `multipart/x-mixed-replace` hình video từ bên kia (`409 NO_VIDEO` nếu chưa có khung). |
| `GET` | `/api/live/snapshot/:call_id` | Bearer hoặc Cookie | `:call_id`, `?account=` | Lấy ảnh JPEG mới nhất từ luồng video cuộc gọi (hạn mức gấp 3 lần API thường). |
| `GET` | `/api/live/cast/:call_id/:file` | Token nội bộ từng cuộc gọi trong path | `:call_id`, `:file` (`<token>.mp3`) | Luồng MP3 tương thích ngược cho thiết bị Cast nội bộ (không khuyến nghị sử dụng mới). |

## Định dạng chữ tự động khi gửi qua API / automation

Các route gửi/sửa chữ hỗ trợ thêm 3 tham số tuỳ chọn để gateway tự dựng `entities` chuẩn UTF-16 cho TDLib (bỏ trống toàn bộ ⇒ giữ nguyên hành vi cũ, không tự định dạng):
- `POST /api/chats/{chat_id}/send`, `POST /api/messages/edit`, `POST /api/send_message`, `POST /api/send_media`, `POST /api/chats/{chat_id}/send_album` (trong JSON body; `send_album` đặt ở gốc hoặc từng phần tử `items[]`).
- `POST /api/chats/{chat_id}/send_file` (trong query string cùng `caption`).

| Tham số | Giá trị | Ý nghĩa |
| --- | --- | --- |
| `format` | `"plain"` (mặc định) \| `"markdown"` | Phân tích markdown nhẹ: `**đậm**`, `*nghiêng*` / `_nghiêng_`, `__gạch chân__`, `~~gạch giữa~~`, `` `mã` ``, ```` ```khối mã``` ````, `\|\|spoiler\|\|`, dòng đầu `> ` trích dẫn, `[chữ](url)` (chỉ nhận `http://`, `https://`, `tg://`), escape bằng `\`. Markdown không đóng hoặc URL lạ giữ nguyên chữ, không mất nội dung. |
| `auto_emphasis` | `false` (mặc định) \| `true` | Tự in đậm con số kèm đơn vị dính liền theo cú pháp số (vd `29°C`, `-5°C`, `75%`, `3 giờ`, `15 phút`, `12,5 mm`, `30 km/h`, `29 độ C`, `2.5 kg`), bỏ qua vùng đã có định dạng/mã và URL. |
| `ai_markup` | chuỗi markdown | Chuỗi đã gắn dấu định dạng (và tối đa 16 emoji trang trí) do trợ lý hội thoại/AI sinh quanh chữ gốc. Gateway kiểm tra chữ gốc giữ nguyên sau khi bỏ dấu; sai lệch trả `422 AI_MARKUP_MISMATCH`. |

Giới hạn áp dụng khi dựng định dạng: nội dung sau bỏ dấu `≤ 4096` đơn vị mã UTF-16, tối đa `100` entity; không dùng đồng thời với `parse_mode` khác `plain` hoặc `entities` khác rỗng (`400 BAD_REQUEST`).

Kiểm hình dạng JSON không cần máy chủ: `python3 tests/contract/kiem_hop_dong.py --self-test`.

## Bảo mật

- **Xác thực hai lớp (API & Web UI):** Tích hợp tự động hoá dùng `Authorization: Bearer <GATEWAY_TOKEN>` (so sánh thời gian hằng qua `subtle::ConstantTimeEq`). Giao diện web chỉ dùng `GATEWAY_TOKEN` một lần lúc thiết lập ban đầu, sau đó dùng mật khẩu băm **PBKDF2-HMAC-SHA256** (210.000 vòng, muối 16 byte ngẫu nhiên) và cookie phiên `tggw_session` (`HttpOnly`, `SameSite=Lax`, `Path=/`, tự bật `Secure` trên HTTPS hoặc khi `TGGW_UI_COOKIE_SECURE=1`).
- **Chống CSRF & XSS:** Mọi yêu cầu ghi (`POST`, `PUT`, `DELETE`, `PATCH`) qua cookie đều phải qua `csrf_valid` (`X-Requested-With: fetch` hoặc khớp `Origin`/`Host`). Giao diện `web/app.html` và `web/index.html` không dùng `innerHTML`/`localStorage`/`sessionStorage`, kèm CSP nghiêm ngặt (`default-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`) và thư viện QR nhúng cục bộ (`web/qrcode.min.js`).
- **Cô lập tệp & chống leo thang đường dẫn:** Mọi tệp tải về từ Telegram (`/api/file/:file_id`, `/api/calltrace/:call_id`) đều gắn `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox`, `Cross-Origin-Resource-Policy: same-origin` và ép tải xuống `attachment` nếu không phải ảnh raster/âm thanh/video an toàn; đọc tệp bằng `openat` + `O_NOFOLLOW` chống symlink.
- **Bảo vệ thao tác phá huỷ & che bí mật:** Đổi hồ sơ, giảm hạn lưu trữ/chuyển chế độ `stream`, xoá bộ nhớ đệm, bật/đổi tự xoá tin nhắn, xoá lịch sử, xoá hội thoại và rời nhóm đều bắt buộc `confirm: true`. Bí mật, số điện thoại, URL có token và đường dẫn tệp nhạy cảm đều được lọc trước khi ghi nhật ký `stderr`, `calltrace` hoặc gửi về Home Assistant.

Chi tiết xem [docs/BAO-MAT.md](docs/BAO-MAT.md).

## Giấy phép

- **Mã nguồn `tele-rust`:** Giấy phép **MIT** — xem [LICENSE](LICENSE).
- **Thành phần bên thứ ba (`NOTICE.md`):**
  - **NTgCalls `3.0.7`** (`https://github.com/pytgcalls/ntgcalls`): **LGPL-3.0-or-later** (liên kết tĩnh qua crate `ntgcalls = "=3.0.7"`). Bản phát hành GitHub Release chỉ đính kèm mã nguồn; ảnh Docker chứa đầy đủ `/licenses/LICENSE`, `/licenses/NOTICE.md` kèm hướng dẫn liên kết lại theo điều khoản LGPL-3.0.
  - **TDLib `1.8.69`** (`https://github.com/tdlib/td`, từ gói `tdjson==1.8.69`): **BSL-1.0** (nạp động lúc chạy qua `TDJSON_LIB`).
  - **qrcode-generator `1.4.4`** (`web/qrcode.min.js`): **MIT** — xem [web/QRCODE-LICENSE.txt](web/QRCODE-LICENSE.txt).
  - **FFmpeg** & **edge-tts** (tuỳ chọn `WITH_TTS=1`): phân phối trong ảnh Docker theo giấy phép tương ứng (LGPL/GPL và LGPL-3.0).

## Tài liệu & kiểm thử

- [docs/API.md](docs/API.md) — Đặc tả chi tiết hợp đồng REST, mã lỗi, tham số và danh mục `/api/do/:name`.
- [docs/TRIEN-KHAI.md](docs/TRIEN-KHAI.md) — Hướng dẫn triển khai Docker, Add-on HA, tự dựng, reverse proxy và sao lưu.
- [docs/BAO-MAT.md](docs/BAO-MAT.md) — Phân tích mô hình bảo mật thực tế trong mã nguồn.
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — Nhật ký thay đổi và ghi chú phiên bản `v0.7.1`.
- [addon/tele-rs/DOCS.md](addon/tele-rs/DOCS.md) — Hướng dẫn cấu hình chi tiết Add-on Home Assistant.
- Kiểm thử cục bộ:
  - `cargo test --release` — toàn bộ unit/integration test Rust.
  - `node --test tests/readme-matches-code.test.cjs` — kiểm README khớp biến môi trường và route trong mã nguồn.
  - `python3 tests/contract/kiem_addon_ci.py` — kiểm cấu trúc add-on, Dockerfile và GitHub Actions.
  - `python3 tests/contract/kiem_web.py` và `python3 tests/contract/kiem_hop_dong.py --self-test` — kiểm tĩnh HTML/JS và hình dạng JSON.
