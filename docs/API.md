# Tài liệu HTTP REST API — tele-rust (`v0.7.5`)

Tài liệu này tổng hợp hợp đồng HTTP REST thực tế được hiện thực trong `src/rest.rs`, `src/ui_auth.rs`, `src/login_http.rs`, `src/account_router.rs`, `src/chat/` và `src/calls.rs`.

## 1. Xác thực, CSRF và hạn mức

- **Đường dẫn công khai (không cần Bearer/Cookie):** `GET /health`, giao diện HTML/JS tĩnh (`/`, `/app.html`, `/ui/app.html`, `/index.html`, `/ui/`, `/ui/index.html`, `/ui/qrcode.min.js`), `GET /ui/api/auth/status`, `POST /ui/api/auth/setup`, `POST /ui/api/auth/login`.
- **Xác thực Bearer:** Gửi header `Authorization: Bearer <GATEWAY_TOKEN>` (so sánh thời gian hằng). Không cần kiểm tra CSRF.
- **Xác thực Cookie trình duyệt:** Cookie `tggw_session` (`HttpOnly; SameSite=Lax; Path=/`, kèm `Secure` trên HTTPS hoặc `TGGW_UI_COOKIE_SECURE=1`). Mọi yêu cầu `POST`/`PUT`/`DELETE`/`PATCH` bằng cookie phải có `X-Requested-With: fetch` hoặc header `Origin` khớp cùng giao thức và `Host`.
- **Token media cuộc gọi (`?t=`):** Riêng `GET /api/live/audio/out/:call_id?t=<token_32_ky_tu>` sử dụng token 192 bit (hạn tối đa 600 giây) lấy từ `POST /api/live/media`. Không truyền `GATEWAY_TOKEN` trên query.
- **Giới hạn kích thước & tốc độ:**
  - Thân JSON chuẩn tối đa `400.000` byte; thân trống được hiểu là `{}`.
  - Tải tệp `POST /api/chats/:chat_id/send_file` và tải xuống `/api/file/:file_id`: tối đa `64 MiB`.
  - Đẩy PCM mic `POST /api/live/audio/in/:call_id`: tối đa `64 KiB`, timeout đọc thân `5` giây, tối đa `8` slot đồng thời.
  - Hạn mức điều khiển mặc định: `120` yêu cầu/phút (`RATE_LIMIT_PER_MINUTE`); `/api/live/snapshot/:call_id` có hạn mức riêng gấp `3` lần (`360`/phút); `/api/live/audio/in/:call_id` không trừ hạn mức điều khiển (chỉ giới hạn 30 POST/giây, 32.000 byte/giây với burst 64 KiB theo từng cuộc gọi).

## 2. Định dạng phản hồi lỗi chuẩn

- **Lỗi nghiệp vụ (`GwError`):** trả JSON dạng:
  ```json
  {
    "error": {
      "code": "BAD_REQUEST",
      "message": "Mô tả lỗi tiếng Việt",
      "retry_after_seconds": 30
    }
  }
  ```
  (`retry_after_seconds` chỉ xuất hiện khi có giới hạn tốc độ như `FLOOD_WAIT` hoặc `LOGIN_RATE_LIMITED`).
- **Lỗi kiểm tra đường dẫn của router trước auth (tương thích FastAPI):**
  - Sai đường dẫn: HTTP `404` → `{"detail":"Not Found"}`
  - Sai phương thức HTTP: HTTP `405` → `{"detail":"Method Not Allowed"}`
  - Sai kiểu số nguyên cho `:chat_id`, `:file_id` hoặc `?since=`: HTTP `422` → `{"detail":[{"type":"int_parsing",...}]}`
- **Mã lỗi nghiệp vụ thường gặp:** `UNAUTHORIZED` (401), `INVALID_CREDENTIALS` (401), `CSRF_FORBIDDEN` (403), `MEDIA_TOKEN` (403), `NOT_FOUND` (404), `ALREADY_CONFIGURED` (409), `CALL_BUSY` (409), `CALL_NOT_ACTIVE` (409), `NO_VIDEO` (409), `NO_DISCUSSION` (409), `STORAGE_BUSY` (409), `STORAGE_CACHE_PATH` (409), `STORAGE_POLICY_CHANGED` (409), `AUDIO_TOO_LARGE` / `FILE_TOO_LARGE` (413), `INVALID_PHONE` / `INVALID_STORAGE_SETTINGS` / `INVALID_APPEARANCE_SETTINGS` / `REPORT_NEEDS_DETAILS` (422), `RATE_LIMITED` / `LOGIN_RATE_LIMITED` / `LOGIN_LOCKED` / `FLOOD_WAIT` / `AUDIO_BACKPRESSURE` / `AUDIO_LISTENERS` (429), `BROWSER_VIDEO_UNAVAILABLE` / `TTS_UNAVAILABLE` / `AUDIO_UNAVAILABLE` (501), `ACCOUNT_OFFLINE` / `ACCOUNT_CLOSING` / `STORAGE_STATS` / `STORAGE_CLEANUP` / `CALL_RUNTIME` (503), `TDLIB_TIMEOUT` / `TELEGRAM_TIMEOUT` (504).

## 3. Chọn tài khoản ở chế độ đa tài khoản

- Bỏ trống `?account=` (hoặc trường JSON `"account"` cho `/api/live/*`, `/api/send_*`): gateway chọn tài khoản `online` đầu tiên theo thứ tự đăng ký.
- Riêng các đường dẫn gắn phạm vi tài khoản gồm `/api/profile`, `/api/settings/storage`, `/api/settings/appearance`, `/api/storage/*`, `/api/chats/:chat_id/{info,settings,clear_history,delete,leave,members}`: nếu có từ `2` tài khoản trở lên, **bắt buộc** chỉ định `?account=<phone>` (thiếu trả `400`).

## 4. Nhóm đường dẫn chi tiết

### 4.1. Trạng thái & xác thực web (`/health`, `/ui/api/auth/*`)
- `GET /health` → `{"status":"ok"|"degraded","version":"0.7.1","uptime_seconds":12,"accounts_total":1,"accounts_online":1,"mock_mode":false}`.
- `GET /ui/api/auth/status` → `{"da_thiet_lap":bool,"dang_nhap":bool,"nguoi_dung":string|null}`.
- `POST /ui/api/auth/setup` → thân `{"token":"...","ten":"admin","mat_khau":"mat-khau-du-10-ky-tu"}`.
- `POST /ui/api/auth/login` → thân `{"ten":"admin","mat_khau":"...","ghi_nho":true}` (khoá 15 phút sau 5 lần sai cùng IP+user).
- `POST /ui/api/auth/logout`, `POST /ui/api/auth/logout-all`, `POST /ui/api/auth/password` (`{"cu":"...","moi":"..."}`).

### 4.2. Tài khoản Telegram (`/ui/api/accounts*`, `/api/accounts`, `/api/me`, `/api/profile`)
- `POST /ui/api/accounts` (`{"phone":"+84900000000"}`) → `{"login_id":"..."}` rồi nộp `POST /ui/api/accounts/:login_id/code` (`{"code":"12345"}`) và `.../password` (`{"password":"..."}`) nếu cần 2FA.
- `POST /ui/api/accounts/qr` (`{"phone":"+84900000000"}`) → `{"login_id":"...","qr_link":"tg://..."}`; poll `GET /ui/api/accounts/:login_id` hoặc xác nhận `POST .../qr-confirm`.
- `DELETE /ui/api/accounts/:login_id` (`:login_id` là số điện thoại URL-encoded) → chờ TDLib đóng phiên (`Closed`) rồi chuyển vào `tdlib/.da-xoa-*`.
- `GET /api/accounts` → `{"tai_khoan":[{"phone":"...","display_name":"...","status":"online"|"offline"}]}`.
- `GET /api/profile` / `POST /api/profile`: cập nhật yêu cầu `confirm: true` cùng ít nhất một trong `first_name` (1–64), `last_name` (0–64), `bio` (0–70), `username` (rỗng hoặc 5–32 ký tự `a-z0-9_`).

### 4.3. Hội thoại, tin nhắn & thông tin chat (`/api/chats*`, `/api/messages/*`, `/api/search`, `/api/resolve`, `/api/translate`)
- `GET /api/chats?limit=60` (tối đa `200`).
- `GET /api/chats/:chat_id/messages?before=0&limit=30&q=` (có `q` tối đa 128 ký tự sẽ gọi `searchChatMessages`).
- `GET /api/chats/:chat_id/messages/:message_id/comments?from=0&limit=30`: lấy bình luận của bài đăng kênh từ nhóm thảo luận liên kết (`{discussion_chat_id, thread_id, root_message_id, messages, has_more}`; trả `409 NO_DISCUSSION` nếu kênh không có nhóm thảo luận).
- `POST /api/chats/:chat_id/messages/:message_id/comments`: gửi bình luận vào nhóm thảo luận (`{"text": "...", "entities": [...], "reply_to": 0}`; hỗ trợ `format`/`auto_emphasis`/`ai_markup`, không yêu cầu `confirm`, trả `409 NO_DISCUSSION` nếu không có nhóm thảo luận).
- `GET /api/chats/:chat_id/info`: trả `chat_id`, `title`, `kind` (`private`/`secret`/`group`/`channel`), `unread`, `marked_unread`, `pinned`, `archived`, `auto_delete_seconds`, `notification`, cùng thông tin thành viên/quyền/liên kết mời (chỉ trả `invite_link` nếu `is_admin == true`).
- `GET /api/chats/:chat_id/members?q=&limit=50`: thành viên nhóm/siêu nhóm (lọc theo `q`, giới hạn tối đa `50`).
- `POST /api/chats/:chat_id/settings`: nhận một hoặc nhiều khoá trong `["notification", "auto_delete", "pin", "archive", "mark_unread"]`. Riêng `auto_delete` yêu cầu `{"seconds": 0 | 86400*N (1..365), "confirm": true}`.
- `POST /api/chats/:chat_id/clear_history`: `{"confirm": true, "revoke": false, "remove_from_list": false}`.
- `POST /api/chats/:chat_id/delete`: `{"confirm": true, "revoke": false}` (nhóm/kênh sẽ gọi `leaveChat`; chat riêng gọi `deleteChatHistory` với `remove_from_chat_list: true`).
- `POST /api/chats/:chat_id/leave`: `{"confirm": true}`.
- `POST /api/chats/:chat_id/send`: `{"text": "...", "reply_to": 0, "parse_mode": "plain"|"markdown"|"html", "entities": [{"offset":0,"length":3,"type":"bold"}]}` (tối đa 100 entity, UTF-16 offset/length, hỗ trợ `bold`, `italic`, `underline`, `strikethrough`, `code`, `pre`, `spoiler`, `blockquote`; không dùng `parse_mode` khác `plain` cùng `entities`).
- `POST /api/chats/:chat_id/send_file?kind=document&name=file.txt&caption=&parse_mode=plain&reply_to=0`: thân thô nhị phân ≤64 MB; `kind` ∈ `photo|video|voice|audio|document` (`voice` tự đổi sang Ogg/Opus bằng `ffmpeg`).
- `POST /api/chats/:chat_id/send_album`: `{"items":[{"kind":"photo","data_b64":"...","name":"...","caption":"..."}], "reply_to":0, "parse_mode":"plain"}` (2–10 mục, không trộn ảnh/video với tài liệu hoặc âm thanh).
- `POST /api/chats/:chat_id/read`: `{"message_ids":[...]}` (tối đa 100 ID).
- `POST /api/messages/edit` (`chat_id`, `message_id`, `text`, `parse_mode`, `entities`), `POST /api/messages/delete` (`chat_id`, `message_ids`, `revoke`), `POST /api/messages/forward` (`to_chat_id`, `from_chat_id`, `message_ids`), `POST /api/messages/react` (`chat_id`, `message_id`, `emoji`, `on`).

### 4.4. Danh mục dịch vụ `/api/do/:name` (`src/chat/services.rs`)
Gateway hỗ trợ cố định 30 tên hành động dịch vụ cho tự động hoá Home Assistant (không cho phép chuyển tiếp JSON tuỳ ý sang TDLib):

1. `set_mute` (`chat_id`, `mute_for` `-1..31622400`)
2. `pin_chat` (`chat_id`, `pinned`, `archived`)
3. `archive_chat` (`chat_id`, `archive`)
4. `mark_chat_unread` (`chat_id`, `unread`)
5. `set_auto_delete` (`chat_id`, `seconds`)
6. `send_typing` (`chat_id`, `action`: `typing` | `recording_voice` | `cancel`)
7. `delete_chat_history` (`chat_id`, `confirm: true`, `remove_from_list`, `revoke`)
8. `rename_chat` (`chat_id`, `title` 1–128 ký tự)
9. `leave_chat` (`chat_id`)
10. `get_chat_info` (`chat_id`)
11. `pin_message` (`chat_id`, `message_id`, `notify`, `only_for_self`)
12. `unpin_message` (`chat_id`, `message_id`)
13. `send_poll` (`chat_id`, `question` 1–300, `options` 2–10, `anonymous`, `multiple`, `quiz_answer`, `explanation`)
14. `vote_poll` (`chat_id`, `message_id`, `options` đánh số từ `1`)
15. `stop_poll` (`chat_id`, `message_id`)
16. `find_user` (đúng một trong `phone` hoặc `username`)
17. `get_user_info` (`user_id`)
18. `add_contact` (`user_id`, `first_name`, `last_name`, `phone`, `share_phone`)
19. `remove_contact` (`user_id`)
20. `block_user` (`user_id`)
21. `unblock_user` (`user_id`)
22. `create_group` (`user_ids`, `title`)
23. `add_chat_member` (`chat_id`, `user_id`, `forward_limit`)
24. `remove_chat_member` (`chat_id`, `user_id`)
25. `create_invite_link` (`chat_id`, `name`, `expire_seconds`, `member_limit`, `join_request`)
26. `join_chat` (`invite_link` bắt đầu bằng `t.me/` hoặc `telegram.me/`)
27. `get_me`
28. `set_name` (`first_name`, `last_name`)
29. `set_bio` (`bio`)
30. `set_username` (`username`)

### 4.5. Cài đặt lưu trữ & giao diện (`/api/settings/*`, `/api/storage/*`)
- `GET /api/settings/storage` và `POST /api/settings/storage`: `mode` (`local` | `stream`), `keep` (`private`, `groups`, `channels`: `null` | `86400` | `604800` | `2678400`), `exceptions` (`[{chat_id, keep}]`), `max_cache_bytes` (`null` hoặc số nguyên ≤2^53-1), `stream_ttl_s`, `auto_download` (`private`/`groups`/`channels` × `photo`/`video`/`file`/`voice`/`video_note`/`gif`: `{on, max_bytes}`). Yêu cầu `confirm: true` nếu chuyển `local → stream` hoặc thu hẹp thời hạn/dung lượng/bỏ ngoại lệ Mãi mãi.
- `POST /api/storage/clear`: bắt buộc `confirm: true`, `scope` là `"all"`, `"chat"` (kèm `chat_id`), hoặc `"type"` (kèm `file_types` như `photo`, `video`, `file`, `music`, `other` hoặc tên `fileType*`). `all` và `type` tự bỏ qua các hội thoại đang giữ «Mãi mãi».
- `GET /api/settings/appearance` và `POST /api/settings/appearance`: bắt buộc có `font_family` (`system`|`sans`|`serif`|`mono`), `font_size` (`12..=20`), `theme` (`system`|`light`|`dark`|`midnight`|`telegram`|`green`|`purple`|`orange`|`pink`|`high-contrast`), `bubble_style` (`classic`|`rounded`|`compact`), `accent` (`blue`|`green`|`purple`|`orange`|`pink`|`teal`); tuỳ chọn `line_height` (`compact`|`normal`|`relaxed`), `chat_bg` (`plain`|`pattern`|`gradient`), `code_theme` (`auto`|`light`|`dark`).

### 4.6. Cuộc gọi trực tiếp & media (`/api/live/*`)
- `POST /api/live/call`: `{"peer":"@username"|"+84..."|user_id, "video":false, "mic":"browser"|"silence"|"test"|url, "camera":"black"|"test"|url, "speaker":"browser"|""|url, "playback":"file"|"external"|"shell"}`.
- `POST /api/live/media`:
  - Lấy luồng nghe trên trình duyệt: `{"id":"<call_id>", "action":"audio_out"}` → `{"audio_out":{"url":"/api/live/audio/out/<id>?t=...","content_type":"audio/wav","expires_in":600}}`.
  - Đổi nguồn gửi đi trong cuộc gọi: `{"id":"<call_id>", "mic":..., "mic_muted":bool, "mic_format":"auto"|"s16le16k", "camera":..., "camera_off":bool, "video":true}`. (`camera:"browser"` khi bật video trả `501 BROWSER_VIDEO_UNAVAILABLE`).
- `POST /api/live/say`: `{"id":"<call_id>", "text":"Xin chào", "voice":"vi-VN-HoaiMyNeural", "rate":"+0%"}` hoặc truyền `audio_b64` (tối đa 28 MB base64).
- `POST /api/live/ha_devices`: `{"mic":[{"id":"...","name":"..."}], "speaker":[...], "camera":[...]}` (tối đa 64 mục mỗi nhóm; URL kiểm tra định dạng hợp lệ và loại bỏ ký tự shell).
