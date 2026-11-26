# Mô hình bảo mật — tele-rust (`v0.7.40`)

Tài liệu mô tả các cơ chế bảo vệ được triển khai trực tiếp trong mã nguồn Rust và giao diện web của `tele-rust`.

## 1. Xác thực và quản lý phiên

### Token máy-gọi-máy (`GATEWAY_TOKEN`)
- Mọi lời gọi từ Home Assistant qua header `Authorization: Bearer <token>` được kiểm tra bằng so sánh thời gian hằng (`subtle::ConstantTimeEq` trong `src/rest.rs`).
- Nếu biến `GATEWAY_TOKEN` rỗng, lệnh `tggw serve` từ chối khởi động ngay (`src/rest.rs::serve_config_from_env`).
- Giao diện web **không lưu** `GATEWAY_TOKEN` vào `localStorage`/`sessionStorage`; token chỉ dùng một lần ở bước `/ui/api/auth/setup` để thiết lập tài khoản web đầu tiên.

### Tài khoản đăng nhập web (`src/ui_auth.rs`)
- Mật khẩu (tối thiểu 10 ký tự, tối đa 1024, không trùng tên đăng nhập) được băm bằng **PBKDF2-HMAC-SHA256** với `210.000` vòng lặp (`ring::pbkdf2`) và muối ngẫu nhiên 16 byte.
- Token phiên (32 byte ngẫu nhiên từ `SystemRandom`) chỉ lưu bản băm SHA-256 trên đĩa (`ui_auth.json`, quyền `0600`, thư mục `0700`, khoá tệp độc quyền `flock` và mở với `O_NOFOLLOW` chống tấn công symlink).
- Cookie `tggw_session` luôn gắn `HttpOnly; SameSite=Lax; Path=/`, tự gia hạn khi còn dưới nửa thời hạn (tối đa 90 ngày khi bật ghi nhớ) và bật `Secure` khi chạy qua HTTPS hoặc đặt `TGGW_UI_COOKIE_SECURE=1`.
- Chống thử sai mật khẩu: tối đa 5 lần thất bại trong cửa sổ 15 phút theo cặp `(IP, username)`, bảng ghi nhận giới hạn 4096 khoá tránh tràn bộ nhớ, đồng thời tách `Semaphore(2)` cho tác vụ băm mật khẩu để request gửi thân chậm (slowloris) không làm treo quá trình xác thực.
- Xác thực đa tài khoản Telegram (`src/login_http.rs`): khoá 15 phút (429 `LOGIN_LOCKED`) sau 3 lần sai OTP hoặc mật khẩu 2FA (không thể lách bằng cách tạo `login_id` mới cho cùng số điện thoại); đóng client bỏ dở sau 10 phút.

## 2. Chống CSRF, XSS và cô lập tài nguyên

- **Kiểm tra CSRF (`src/rest.rs::csrf_valid`):** Mọi yêu cầu ghi (`POST`, `PUT`, `DELETE`, `PATCH`) xác thực bằng cookie phải mang `X-Requested-With: fetch` (khi CORS tắt) hoặc có `Origin` khớp chính xác scheme (`https` khi bật `secure_cookie`) và `Host`.
- **Giao diện web an toàn (`web/app.html`, `web/index.html`, `src/web.rs`):**
  - Toàn bộ thao tác DOM dùng `textContent` / `createElement`, không sử dụng `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `localStorage`, `sessionStorage`.
  - Mọi liên kết ngoài trong nội dung tin nhắn (`text_link` hoặc auto-link) chỉ chấp nhận `http://` và `https://`, gắn `rel="noopener noreferrer"` và cô lập khỏi định dạng xung quanh.
  - Phản hồi HTML gắn đầy đủ header: `Content-Security-Policy` (`default-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cache-Control: no-store`.
  - Thư viện QR (`web/qrcode.min.js`) được đóng gói nội bộ cùng mã nguồn, không tải mã từ CDN ngoài.
- **Cô lập tệp tải từ Telegram (`src/rest.rs::bytes_resp`, `src/chat/files.rs`):**
  - Mọi tệp qua `/api/file/:file_id` hoặc `/api/calltrace/:call_id` đều gắn `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox` và `Cross-Origin-Resource-Policy: same-origin`.
  - Chỉ các kiểu thụ động (`image/png`, `image/jpeg`, `image/gif`, `image/webp`, `audio/*`, `video/*`) mới hiển thị inline; HTML, SVG, PDF, JS, XML và tệp nhật ký luôn ép `Content-Type: application/octet-stream` (hoặc `text/plain` với `Content-Disposition: attachment`).
  - Đọc tệp luôn xác minh nằm dưới `tdlib/files/` hoặc `calltrace/`, sử dụng `openat` với `O_NOFOLLOW` và giới hạn cứng `64 MiB`.

## 3. Kiểm soát thao tác phá huỷ & bộ nhớ đệm

- **Bắt buộc xác nhận (`confirm: true`):** Áp dụng ở máy chủ đối với cập nhật hồ sơ (`POST /api/profile`), xoá bộ nhớ đệm (`POST /api/storage/clear`), siết chặt chính sách lưu trữ hoặc chuyển sang chế độ `stream` (`POST /api/settings/storage`), bật/đổi tự xoá tin nhắn (`POST /api/chats/:chat_id/settings`), xoá lịch sử (`clear_history`), xoá hội thoại (`delete`), rời nhóm (`leave`) và `delete_chat_history` trong `/api/do/:name`.
- **Bảo vệ bộ nhớ đệm (`src/storage.rs`, `src/account_router.rs`):** Từ chối dọn cache khi đang có cuộc gọi hoạt động (`409 STORAGE_BUSY`), quét đường dẫn cache chống symlink trước khi chạy (`409 STORAGE_CACHE_PATH`), huỷ lượt dọn nếu chính sách thay đổi giữa chừng (`409 STORAGE_POLICY_CHANGED`).

## 4. Bảo mật cuộc gọi, TTS và Webhook

- **Âm thanh trình duyệt (`src/calls/browser.rs`, `src/calls/browser_audio.rs`):**
  - Nguồn mic loopback của NTgCalls chỉ lắng nghe `127.0.0.1` với đường dẫn chứa khoá ngẫu nhiên 192 bit và giới hạn tốc độ 30 POST/giây, 32 KB/giây, tối đa 1 người nghe loopback.
  - Đường ra `/api/live/audio/out/:call_id?t=...` dùng token 192 bit riêng từng cuộc gọi (hết hạn sau 600 giây, tối đa 2 người nghe), tuyệt đối không dùng `GATEWAY_TOKEN` trên query string.
- **Lọc đầu vào lệnh ngoài (`src/calls/sources.rs`, `src/calls/catalog.rs`, `src/config.rs`):**
  - Nguồn `mic`, `camera`, `speaker`, `ha_devices`, `TTS_VOICE`, `TTS_RATE` đều qua whitelist ký tự/scheme chặt chẽ; mọi tham số truyền vào lệnh `ffmpeg` đều được trích dẫn shell an toàn (`quote()`), còn `edge-tts` truyền tham số bằng mảng `Command::args` (không qua shell).
- **Chống SSRF webhook (`src/events/webhook.rs`):** Chặn IP link-local (như `169.254.0.0/16` / `fe80::/10`) ở cả kiểm tra URL lẫn bước phân giải DNS (`FilteredDns`), tắt chuyển hướng HTTP (`Policy::none()`) và tắt proxy hệ thống.
- **Che bí mật trong log (`src/calls/trace.rs`, `src/logging.rs`):** Trước khi ghi ra `stderr` hay `calltrace/`, bộ lọc `mask()` thay thế URL, `Bearer ...`, token/api_hash/password/cookie, số điện thoại, địa chỉ IP và giá trị các biến môi trường bí mật (`GATEWAY_TOKEN`, `TG_UI_PASSWORD`, `TG_API_HASH`, `TG_PHONE`, `TG_API_ID`, `HA_WEBHOOK_TOKEN`) thành `<ẩn>`.

## 5. Vận hành container an toàn

- Ảnh Docker chạy bằng người dùng không có quyền root (`tggw`, UID/GID `10001`).
- Không nhúng bất kỳ khoá, mật khẩu hay dữ liệu cá nhân nào vào ảnh; mọi bí mật được truyền qua biến môi trường hoặc tệp cấu hình `/data/options.json` khi chạy.
- Nhật ký khởi động của `docker/entrypoint.sh` không bao giờ in giá trị biến cấu hình ra `stdout`/`stderr`.
