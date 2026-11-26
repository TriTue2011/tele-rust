# tele-rust

[![CI](https://github.com/TriTue2011/tele-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/TriTue2011/tele-rust/actions/workflows/ci.yml)
[![Image](https://github.com/TriTue2011/tele-rust/actions/workflows/image.yml/badge.svg)](https://github.com/TriTue2011/tele-rust/actions/workflows/image.yml)
[![Add-on](https://github.com/TriTue2011/tele-rust/actions/workflows/addon.yml/badge.svg)](https://github.com/TriTue2011/tele-rust/actions/workflows/addon.yml)

Gateway **Telegram** (tài khoản người dùng) cho **Home Assistant**: nhắn tin, gọi thoại/video và đẩy sự kiện về HA, kèm giao diện web. Viết bằng Rust.

> **Tuyên bố miễn trừ trách nhiệm:** Đây là **dự án không chính thức, không liên kết, không được tài trợ hay xác nhận bởi Telegram**. Phần mềm tương tác với mạng Telegram thông qua TDLib (`libtdjson` 1.8.69, nạp động lúc chạy) và NTgCalls (`3.0.7`, liên kết tĩnh). Người dùng tự chịu trách nhiệm khi triển khai và sử dụng với tài khoản của mình.

- **Phiên bản hiện tại:** `v0.7.37` (`Cargo.toml` và `addon/tele-rs/config.yaml`)
- **Ảnh Docker (`linux/amd64`, `linux/arm64`):** `ghcr.io/tritue2011/tele-rust:latest` (cùng thẻ `:v0.7.37`, `:0.7.37`, `:sha-<commit>`)
- **Chưa có app macOS native** (chi tiết ở [docs/TRIEN-KHAI.md](docs/TRIEN-KHAI.md)) — ảnh Docker `arm64` vẫn chạy được trên Apple Silicon qua Docker Desktop.

## 1. Chuẩn bị: lấy các thông tin cần

> Dùng **tài khoản Telegram phụ**, không dùng tài khoản chính. Một tài khoản chỉ nên đăng nhập ở **một nơi**.

| Cần | Lấy ở đâu | Dùng cho |
| --- | --- | --- |
| `api_id`, `api_hash` | Xem ngay dưới | Biến `TG_API_ID`, `TG_API_HASH` |
| Số điện thoại tài khoản Telegram phụ | SIM/số của bạn (kèm mã nước, ví dụ `+84…`) | Đăng nhập trên giao diện web ở bước 3 |
| `GATEWAY_TOKEN` | Tự sinh: `openssl rand -hex 32` | Mật khẩu chung giữa gateway và Home Assistant; cũng dùng để tạo tài khoản web lần đầu |
| Mật khẩu web (tuỳ chọn) | Tự đặt, tối thiểu 10 ký tự | `TG_UI_USER` + `TG_UI_PASSWORD` để tạo sẵn tài khoản web (bỏ qua thì tạo bằng token ở bước 3) |

**Lấy `api_id` và `api_hash` (một lần):**

1. Vào <https://my.telegram.org>, nhập số điện thoại (tài khoản phụ) rồi nhập mã Telegram gửi về.
2. Chọn **API development tools**.
3. Điền **App title**, **Short name** (tuỳ ý), **Platform** = *Desktop* → **Create application**.
4. Chép **App api_id** (một số) và **App api_hash** (chuỗi 32 ký tự). **Giữ bí mật `api_hash` và `GATEWAY_TOKEN`.**

## 2. Cài đặt gateway

### Cách A: Add-on Home Assistant (HAOS / Supervised)

1. **Cài đặt → Tiện ích bổ sung → Cửa hàng tiện ích bổ sung → ba chấm góc trên → Kho lưu trữ**, thêm `https://github.com/TriTue2011/tele-rust`.
2. Cài **Tele Rust**. Ở tab **Cấu hình** điền `tg_api_id`, `tg_api_hash`, `gateway_token` (tuỳ chọn `tg_ui_password`), rồi khởi động.
3. Nếu kho được chuyển sang chế độ riêng tư, xem mục private trong [docs/TRIEN-KHAI.md](docs/TRIEN-KHAI.md).

### Cách B: Docker

Container chạy dưới người dùng không root UID/GID `10001`, dữ liệu ở `/data`, cổng mặc định `3334`.

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

> **Home Assistant chạy ở máy khác?** Đổi `127.0.0.1:3334:3334` thành `IP_LAN_CỦA_MÁY_NÀY:3334:3334` và chỉ cho phép IP của Home Assistant ở tường lửa.

### Cách C: Docker Compose

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

**Mic và webcam của trình duyệt cần HTTPS:** gateway nhận cả `http://` lẫn `https://` trên cùng cổng (chứng chỉ tự ký, trình duyệt hỏi tin cậy một lần) — Docker, Docker Compose và add-on đều dùng được, không cần đổi cổng. Mở `https://<máy>:<cổng>` khi cần dùng mic/webcam.

Đặt sau reverse proxy HTTPS, tự dựng từ mã nguồn, sao lưu: xem [docs/TRIEN-KHAI.md](docs/TRIEN-KHAI.md).

## 3. Tạo gateway: đăng nhập tài khoản Telegram

1. Mở `http://<máy chạy gateway>:3334/ui/`.
2. **Lần đầu:** tạo tài khoản web bằng `GATEWAY_TOKEN` + tên đăng nhập + mật khẩu (tối thiểu 10 ký tự). Nếu đã đặt `TG_UI_USER` và `TG_UI_PASSWORD` thì tài khoản đã được tạo sẵn, chỉ cần đăng nhập.
3. Thêm tài khoản Telegram: nhập **số điện thoại** → nhập **mã OTP** Telegram gửi về (và **mật khẩu hai lớp** nếu có), hoặc **quét mã QR** bằng ứng dụng Telegram. Thêm nhiều tài khoản được.
4. Kiểm tra: `http://<máy chạy gateway>:3334/health` trả `"status":"ok"` và `accounts_online` từ 1 trở lên.

Quên mật khẩu web: `docker exec -it tele-rust /usr/local/bin/tggw passwd` (đặt lại và thu hồi mọi phiên cũ).

## 4. Kết nối Home Assistant

1. **HACS → ba chấm → Custom repositories**, dán `https://github.com/TriTue2011/Telegram-calling`, **Type = Integration** → **Add** → tìm **Telegram Call** → **Download** → khởi động lại Home Assistant.
2. **Cài đặt → Thiết bị & dịch vụ → Thêm tích hợp → Telegram Call**.
3. Nhập **địa chỉ gateway** (`http://<máy chạy gateway>:3334`) và **`GATEWAY_TOKEN`**, rồi chọn tài khoản Telegram đang online.
4. Có nhiều tài khoản thì lặp lại bước 2–3, **mỗi tài khoản một mục**. Tài khoản offline hoặc đang chờ mã không hiện trong danh sách.

Kết quả: thiết bị **Telegram Call — <tên tài khoản>** (cảm biến cuộc gọi, tin nhắn gần nhất, ô chọn mic/camera/loa) và các dịch vụ `telegram_call.*` để gọi, nghe máy và nhắn tin. Cách dùng từng dịch vụ nằm ở README của tích hợp: <https://github.com/TriTue2011/Telegram-calling>.

## 5. Automation có sẵn

**Blueprint «Telegram Call – Quy tắc cho cuộc gọi đến»** (`telegram_call_incoming_rule.yaml`, kho <https://github.com/TriTue2011/Blueprint>):

[![Nhập blueprint vào Home Assistant](https://my.home-assistant.io/badges/blueprint_import.svg)](https://my.home-assistant.io/redirect/blueprint_import/?blueprint_url=https%3A%2F%2Fgithub.com%2FTriTue2011%2FBlueprint%2Fblob%2Fmain%2Ftelegram_call_incoming_rule.yaml)

Mỗi cuộc gọi Telegram đến kích hoạt automation này (có lượt chạy riêng cho từng cuộc) và nó áp quy tắc cho đúng cuộc gọi đó. Cấu hình gồm bốn nhóm:

1. **Áp dụng cho ai:** một người cụ thể, mọi người mang một thẻ, mọi người trong danh bạ Telegram, hoặc người lạ; lọc theo cuộc thoại hay video.
2. **Tự nghe máy và thiết bị nhà:** tự nghe máy sau bao lâu, nối mic/camera/loa nào, lời chào và xử lý khi bên kia xin video.
3. **Báo ra loa khi có cuộc gọi đến:** loa, câu thông báo, số lần lặp, âm lượng.
4. **Thông báo:** gửi thông báo khi có cuộc gọi.

Quy tắc cụ thể hơn thắng (người &gt; thẻ &gt; danh bạ &gt; người lạ). **Mặc định riêng tư:** chưa có quy tắc thì chỉ đổ chuông; thiết bị nhà chỉ được nối khi bạn tích chọn rõ. Tắt hoặc xoá automation là hết quy tắc. Cần tích hợp Telegram Call từ bản `2026.11.16.2`.

Dùng blueprint: **Cài đặt → Tự động hoá & Cảnh → Tạo tự động hoá → Dùng blueprint**.

## 6. Tài liệu thêm

- [docs/TRIEN-KHAI.md](docs/TRIEN-KHAI.md): reverse proxy HTTPS, add-on riêng tư, tự dựng, sao lưu.
- [docs/THAM-CHIEU.md](docs/THAM-CHIEU.md): lệnh CLI, biến môi trường, route REST, định dạng chữ khi gửi qua API.
- [docs/API.md](docs/API.md), [docs/BAO-MAT.md](docs/BAO-MAT.md), [docs/TINH-NANG.md](docs/TINH-NANG.md), [docs/CHANGELOG.md](docs/CHANGELOG.md).

## Giấy phép

- **Mã nguồn `tele-rust`:** Giấy phép **MIT** — xem [LICENSE](LICENSE).
- **Thành phần bên thứ ba (`NOTICE.md`):**
  - **NTgCalls `3.0.7`** (`https://github.com/pytgcalls/ntgcalls`): **LGPL-3.0-or-later** (liên kết tĩnh qua crate `ntgcalls = "=3.0.7"`). Bản phát hành GitHub Release chỉ đính kèm mã nguồn; ảnh Docker chứa đầy đủ `/licenses/LICENSE`, `/licenses/NOTICE.md` kèm hướng dẫn liên kết lại theo điều khoản LGPL-3.0.
  - **TDLib `1.8.69`** (`https://github.com/tdlib/td`, từ gói `tdjson==1.8.69`): **BSL-1.0** (nạp động lúc chạy qua `TDJSON_LIB`).
  - **qrcode-generator `1.4.4`** (`web/qrcode.min.js`): **MIT** — xem [web/QRCODE-LICENSE.txt](web/QRCODE-LICENSE.txt).
  - **FFmpeg** & **edge-tts** (tuỳ chọn `WITH_TTS=1`): phân phối trong ảnh Docker theo giấy phép tương ứng (LGPL/GPL và LGPL-3.0).
