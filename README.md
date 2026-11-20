# tele-rust

Gateway **Telegram bằng Rust** (tài khoản người dùng: tin nhắn + gọi thoại/video P2P) cho Home Assistant, thay cho bản
Python (`Telegram-calling/telegram-gateway`). TDLib 1.8.69 (`libtdjson`, gọi qua JSON, mở lúc chạy) + NTgCalls 3.0.7 (liên kết tĩnh).

> **Trạng thái: đang làm (cột mốc 0 xong).** `tggw version` → TDLib 1.8.69; `tggw protocol` → NTgCalls 3.0.7, tgcalls 8/9/12/13.
> Đang viết: client TDLib + đăng nhập, REST đúng hợp đồng bản Python, quản lý cuộc gọi (`src/contract.rs` là hợp đồng giữa các phần).

Dựng: `./build.sh build` (container `rust:1.90` + g++/zlib, ssh tới máy dựng; tắt LTO vì thư viện C++ của NTgCalls không có bitcode).
Lưu ý giấy phép: NTgCalls là LGPL-3.0 liên kết tĩnh — phát hành nhị phân cần xem lại điều khoản liên kết lại. Giấy phép mã của kho này: chưa chọn.
