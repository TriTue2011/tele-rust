# Tham chiếu: lệnh CLI, biến môi trường, route REST, định dạng chữ

Các bảng dưới đây được bài kiểm `tests/readme-matches-code.test.cjs` đối chiếu với `src/config.rs` và `src/rest.rs`: thêm hay bớt biến/route trong mã mà không sửa tệp này thì CI đỏ.

## Lệnh CLI (`tggw`)

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


## Kiểm thử và tài liệu liên quan

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

