# Thông báo bản quyền & giấy phép bên thứ ba (NOTICE)

Dự án **tele-rust** (`https://github.com/TriTue2011/tele-rust`) do tác giả phát hành theo giấy phép **MIT** (xem tệp `LICENSE`).
Khi phân phối hoặc chạy ảnh container Docker của `tele-rust`, các thành phần bên thứ ba sau được sử dụng kèm theo giấy phép riêng của từng thành phần:

## 1. NTgCalls (`3.0.7`) — LGPL-3.0-or-later (liên kết tĩnh)

- **Nguồn:** `https://github.com/pytgcalls/ntgcalls` (crate Rust `ntgcalls = "=3.0.7"`).
- **Giấy phép:** GNU Lesser General Public License v3.0 hoặc mới hơn (`LGPL-3.0-or-later`).
- **Cách liên kết mặc định:** Nhị phân `tggw` trong ảnh phát hành liên kết tĩnh với thư viện C++ của NTgCalls `3.0.7`.

### Hướng dẫn liên kết lại (LGPL-3.0 Relinking Instructions)

Theo điều khoản Mục 4 của giấy phép LGPL-3.0, người dùng có đầy đủ quyền thay thế hoặc liên kết lại nhị phân `tggw` với phiên bản NTgCalls do mình tự biên dịch hoặc chỉnh sửa từ mã nguồn công khai:

1. **Chuẩn bị mã nguồn công khai:**
   - Lấy mã nguồn `tele-rust` tại kho phát hành tương ứng với thẻ `vX.Y.Z` (tarball mã nguồn đi kèm trên GitHub Release hoặc `git clone`).
   - Lấy mã nguồn NTgCalls tại `https://github.com/pytgcalls/ntgcalls` (hoặc fork đã chỉnh sửa của bạn).
2. **Cách A — Liên kết động với thư viện NTgCalls tự dựng (`NTGCALLS_DYLIB`):**
   - Biên dịch `libntgcalls.so` từ mã nguồn NTgCalls theo hướng dẫn của dự án gốc.
   - Đặt biến môi trường trỏ tới thư viện dùng chung khi biên dịch crate `ntgcalls` (tham khảo `build.rs` của crate `ntgcalls` trong cache Cargo hoặc cấu hình `NTGCALLS_DYLIB=/duong/dan/libntgcalls.so`), sau đó chạy:
     ```bash
     cargo build --release --bin tggw
     ```
3. **Cách B — Thay thế crate / bản NTgCalls tĩnh qua `[patch.crates-io]`:**
   - Trỏ crate `ntgcalls` sang bản cục bộ hoặc kho Git đã chỉnh sửa bằng cách thêm vào `Cargo.toml`:
     ```toml
     [patch.crates-io]
     ntgcalls = { path = "/duong/dan/ntgcalls-crate" }
     ```
   - Biên dịch lại nhị phân (cài sẵn `cmake`, `make`, `pkg-config`, `g++`, `zlib1g-dev`):
     ```bash
     cargo build --release --bin tggw
     ```
4. **Dựng lại ảnh Docker với nhị phân đã liên kết lại:**
   - Chạy `docker build -t tele-rust:custom .` từ cây mã nguồn đã chỉnh sửa.
   - Khi khởi chạy container, có thể gắn đè nhị phân tuỳ chỉnh hoặc dùng ảnh vừa dựng.
5. **Kiểm tra sau khi liên kết lại:**
   ```bash
   tggw protocol
   ```
   Lệnh in ra phiên bản `NTgCalls` và danh sách `tgcalls` đang liên kết.

## 2. TDLib (`1.8.69`) — Boost Software License 1.0 (BSL-1.0)

- **Nguồn:** `https://github.com/tdlib/td` (lấy từ wheel nhị phân `tdjson==1.8.69` trên PyPI).
- **Giấy phép:** Boost Software License 1.0 (`BSL-1.0`).
- **Cách liên kết:** `tggw` nạp động thư viện `libtdjson.so` lúc chạy qua `dlopen` (`TDJSON_LIB`). Người dùng có thể tự dựng `libtdjson.so` từ mã nguồn TDLib và trỏ `TDJSON_LIB=/duong/dan/libtdjson.so` mà không cần biên dịch lại `tggw`.

## 3. qrcode-generator (`1.4.4`) — MIT License

- **Tệp trong kho:** `web/qrcode.min.js` (kèm bản gốc giấy phép tại `web/QRCODE-LICENSE.txt`).
- **Tác giả:** Kazuhiko Arase.
- **Giấy phép:** MIT License.

## 4. Công cụ thời gian chạy tuỳ chọn trong ảnh Docker

- **FFmpeg** (cài từ kho Debian `bookworm`): cung cấp chuyển mã âm thanh/video cuộc gọi và tin thoại; phân phối theo giấy phép của gói Debian tương ứng (LGPL/GPL).
- **edge-tts** (cài tuỳ chọn khi `WITH_TTS=1`): cung cấp chuyển văn bản thành giọng nói cho lệnh `/api/live/say` và `/api/send_voice`; phát hành theo giấy phép LGPL-3.0. Có thể tắt hoàn toàn lúc dựng bằng `--build-arg WITH_TTS=0`.
