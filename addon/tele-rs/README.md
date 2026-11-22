# Tele Rust — Add-on Home Assistant

Add-on chạy gateway **tele-rust** (viết bằng Rust, dùng TDLib `1.8.69` + NTgCalls `3.0.7`) để cung cấp HTTP REST API cho tích hợp Home Assistant và giao diện web điều khiển/trò chuyện nhúng.

> **Cảnh báo:** Đây là **dự án không chính thức, không liên kết, không được tài trợ hay xác nhận bởi Telegram**. Người dùng tự chịu trách nhiệm khi triển khai, cấu hình và vận hành với tài khoản của mình.

## Tính năng chính

- Xác thực cookie `HttpOnly` (PBKDF2-SHA256 210.000 vòng, kiểm tra CSRF) cho giao diện web và `Authorization: Bearer` cho Home Assistant.
- Hỗ trợ đa tài khoản Telegram (đăng nhập OTP / QR ngay trên `/ui/index.html`, lưu trữ an toàn phiên đã xoá).
- Quản lý danh bạ, tìm kiếm tin nhắn trong hội thoại, ngăn Thông tin hội thoại, soạn thảo định dạng WYSIWYG, quản lý bộ nhớ đệm TDLib theo hạn mức và ngoại lệ «Mãi mãi».
- Gọi thoại/video P2P qua NTgCalls `3.0.7`, hỗ trợ mic/loa trình duyệt, thiết bị HA và chuyển văn bản thành giọng nói (`edge-tts`).

## Yêu cầu trước khi khởi chạy

Add-on bắt buộc phải cấu hình ba thông số sau trước khi bật:

- `tg_api_id` và `tg_api_hash`: lấy từ trang nhà phát triển Telegram.
- `gateway_token`: chuỗi bí mật ngẫu nhiên (ví dụ sinh bằng `openssl rand -hex 32`) dùng cho Bearer API và thiết lập đăng nhập web lần đầu.

Xem hướng dẫn cài đặt chi tiết và bảng tuỳ chọn đầy đủ trong [DOCS.md](DOCS.md).
