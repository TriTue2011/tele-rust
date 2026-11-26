# Quy tắc tài nguyên (RAM, CPU, kết nối, luồng) cho tele-gw

Áp dụng cho cổng giao tiếp Telegram Rust (`tele-gw` / `tele-rust`) viết bằng Rust (Tokio / Actix-web / TDLib / NTgCalls / Ingress).
Tuân thủ chuẩn quy tắc tài nguyên hệ thống, bảo vệ dịch vụ gọi thoại/video và nhắn tin khỏi rò rỉ RAM, zombie processes, và tệp rác.

---

### 1. Hàng đợi, bộ nhớ đệm, danh sách luôn có trần
- Danh sách cuộc gọi đã xử lý / tombstone (`Actor.handled` và `Actor.handled_order` trong `src/calls.rs`) có trần giới hạn tối đa `256` ID theo cơ chế FIFO. Khi vượt quá 256 phần tử, tự động loại bỏ ID cũ nhất, ngăn chặn `HashSet<i64>` tăng trưởng không giới hạn theo thời gian hoạt động.
- Hàng đợi sự kiện (`EventBridge` trong `src/events.rs`) sử dụng kênh có trần `mpsc::channel(1024)` và kênh phát thanh `broadcast::channel(256)`.

---

### 2. Không chặn vòng lặp Tokio (Async Event Loop)
- Không chạy I/O tệp sự kiện Spool hoặc nạp danh mục thiết bị đồng bộ trên luồng worker Tokio. Tất cả thao tác đọc/ghi đĩa được chuyển sang `tokio::task::spawn_blocking`.
- Luồng NTgCalls binding C++ chạy trong một OS thread riêng với runtime độc lập, không giữ khóa mutex qua các điểm `await`.

---

### 3. Việc nặng có trần chạy cùng lúc & Đóng gói an toàn
- Container Docker và Add-on bắt buộc áp dụng trần tài nguyên chuẩn (đã mô tả trong `docker-compose.example.yml` và `addon/tele-rs/config.yaml`):
  - `mem_limit: 1g` và `memswap_limit: 1g` (bằng nhau, cấm tràn sang swap gây nghẽn I/O máy chủ).
  - `cpus: "1.5"`.
  - `pids_limit: 256` (trần số lượng luồng TDLib + NTgCalls + ffmpeg con).
  - `init: true` (bắt buộc: tini làm PID 1 để thu dọn sạch sẽ tiến trình ffmpeg con mồ côi).
  - `security_opt: [no-new-privileges:true]`.
  - `logging`: driver `json-file` xoay vòng tối đa `max-size: "10m"`, `max-file: "3"`.

---

### 4. Mở nhanh, đóng nhanh & Dọn dẹp tệp tạm thời
- Khi cuộc gọi kết thúc (`Actor::end` trong `src/calls.rs`), hệ thống dọn dẹp triệt để toàn bộ tệp tạm thời âm thanh mang tiền tố `{call_id}_*.pcm` và `{call_id}_*.wav` trong thư mục `live/`.
- Khi gateway khởi động (`CallManager::spawn`), tự động quét và dọn sạch các tệp `.pcm` và `.wav` mồ côi còn sót lại từ lần chạy trước nếu dịch vụ bị dừng đột ngột.
- Mọi tiến trình ffmpeg con được sinh ra qua `Command::new` bắt buộc phải có `.kill_on_drop(true)` để không sinh ra tiến trình zombie khi phiên gọi kết thúc.

---

### 5. Tác vụ nền huỷ được; mỗi phiên một tác vụ
- Mọi tác vụ nền sinh ra bởi `tokio::spawn` phải quản lý vòng đời chặt chẽ.
  *Ví dụ:* `EventBridge` (`src/events.rs`) triển khai `Drop` thực hiện `self.worker.abort()` và `webhook.abort()`, đảm bảo worker nền không bị mồ côi khi bridge bị giải phóng.
- Mỗi cuộc gọi chỉ có một luồng truyền thông (`NTgCalls`); khi cuộc gọi kết thúc, dừng ngay lập tức bằng `MediaOp::Stop(user_id)`.

---

### 6. Nhật ký ghi có chọn lọc, bảo mật thông tin cá nhân
- Nhật ký lỗi không lặp vô tận; che giấu định danh và thông tin cá nhân bằng `mask_peer`, `mask_peer_id`, và `sources::mask(why)`.
- Không in số điện thoại đầy đủ, không ghi bí mật `gateway_token`, `api_hash`, hoặc nội dung giải mã cuộc gọi vào nhật ký.
