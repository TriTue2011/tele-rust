# Tính năng và giới hạn đã biết (tele-rust v0.7.33)

Tài liệu này giữ danh sách tính năng thật và giới hạn của bản hiện tại (trước đây nằm trong README).

## Tính năng thật

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
- **Camera trình duyệt (webcam USB/có sẵn):** `camera: "browser"` nhận khung JPEG từ trình duyệt qua `POST /api/live/video/in/:call_id` (≤256 KiB mỗi khung, tối đa 30 khung/giây), gateway phát lại thành luồng camera của cuộc gọi; chưa có khung nào thì gửi hình đen. Trình duyệt chỉ cho dùng webcam/mic trên HTTPS hoặc localhost: gateway tự bật HTTPS tự ký trên CÙNG cổng (`https://<máy>:3333`, tắt bằng `GATEWAY_HTTPS=0`).
- **macOS: chưa hỗ trợ:** Chưa cung cấp workflow dựng hay nhị phân phát hành cho macOS vì tổ hợp TDLib `1.8.69` và NTgCalls `3.0.7` trên macOS chưa được kiểm chứng thực tế trên máy đích.
- **Kiến trúc `arm64`/`aarch64`:** Ảnh Docker và add-on hiện chỉ dựng cho `linux/amd64` do nhị phân tĩnh NTgCalls `3.0.7` và wheel `tdjson==1.8.69` trên PyPI chỉ có gói `linux/amd64`.
- **Home Assistant Ingress:** Chưa bật Ingress trên add-on vì gateway đặt header `Content-Security-Policy: ... frame-ancestors 'none'`, dùng đường dẫn gốc tuyệt đối `/api/*` và cookie `Path=/`. Truy cập Web UI qua cổng `3334`.
- **Loa Google Cast đã bỏ khỏi luồng khuyến nghị:** Giao diện và tài liệu không còn khuyến nghị thiết lập loa Cast mới (ưu tiên loa trình duyệt `browser` hoặc cửa bộ đàm nội bộ); route `/api/live/cast/:call_id/:file` và bộ giải mã `cast://` trong `src/calls/cast.rs` chỉ giữ lại để tương thích ngược mã hiện có.
- **Các đường REST cũ (`/api/calls`, `/api/call`, `/api/answer_call`, `/api/hangup_call`, `/api/audio/in`, `/api/audio/upload`):** Được đăng ký ở tầng router `src/rest.rs` nhưng `ChatService` trả `404 NOT_FOUND`; mọi điều khiển cuộc gọi dùng nhóm `/api/live/*`.
- **Chế độ giả lập (`GATEWAY_MOCK=1`):** Bản Rust không hỗ trợ giả lập (luôn từ chối khởi động với mã thoát `2`).

