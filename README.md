# tele-rust

Gateway **Telegram bằng Rust** (tài khoản người dùng: tin nhắn + gọi thoại/video P2P) cho Home Assistant, thay cho bản
Python (`Telegram-calling/telegram-gateway`). TDLib 1.8.69 (`libtdjson`, gọi qua JSON, mở lúc chạy) + NTgCalls 3.0.7 (liên kết tĩnh).

> **Trạng thái: đang làm (cột mốc 0 xong).** `tggw version` → TDLib 1.8.69; `tggw protocol` → NTgCalls 3.0.7, tgcalls 8/9/12/13.
> Đang viết: client TDLib + đăng nhập, REST đúng hợp đồng bản Python, quản lý cuộc gọi (`src/contract.rs` là hợp đồng giữa các phần).

Dựng: `./build.sh build` (container `rust:1.90` + g++/zlib, ssh tới máy dựng; tắt LTO vì thư viện C++ của NTgCalls không có bitcode).
Lưu ý giấy phép: NTgCalls là LGPL-3.0 liên kết tĩnh — phát hành nhị phân cần xem lại điều khoản liên kết lại. Giấy phép mã của kho này: chưa chọn.

## Nhiều tài khoản và đăng nhập trên web

`tggw serve` phục vụ giao diện trò chuyện tại `/` và `/ui/app.html`, quản lý tài khoản tại `/ui/index.html`.
Nhập `GATEWAY_TOKEN` để kết nối. Các API `/ui/api/accounts*` đều yêu cầu `Authorization: Bearer …`,
cùng cơ chế xác thực và hạn mức với `/api/*`. Chọn tài khoản trong giao diện trò chuyện, hoặc truyền
`?account=<phone>` vào API; các lệnh `/api/live/*` cũng nhận `account` trong JSON. Khi bỏ trống,
gateway dùng tài khoản online đầu tiên theo thứ tự đăng ký. Tài khoản lạ trả 404 `NOT_FOUND`,
tài khoản đã lưu nhưng chưa kết nối trả 503 `ACCOUNT_OFFLINE`. Feed và sự kiện chứa `account`.

Đăng nhập bằng `POST /ui/api/accounts {phone}`, rồi `POST /ui/api/accounts/{login_id}/code {code}`,
và `/password {password}` nếu trả `needs_password`. Đăng nhập QR bắt đầu tại `POST /ui/api/accounts/qr {phone}`;
`GET /ui/api/accounts/{login_id}` lấy QR mới, trạng thái 2FA hoặc `done`; `/qr-confirm` xác nhận thủ công.
Ba lần nhập mã hoặc mật khẩu sai khóa phiên 15 phút với 429 `LOGIN_LOCKED`. FLOOD_WAIT trả 429 và
`retry_after_seconds` từ Telegram. Client của phiên bỏ dở được đóng sau 10 phút, kể cả phiên bị khóa;
khóa vẫn có hiệu lực tới hết 15 phút. Tạo login_id mới cho cùng số cũng không vượt được khóa.

Sổ tài khoản `accounts.json` được thay thế nguyên tử với quyền 0600. Phiên mới nằm tại
`tdlib/<sha256(phone)[:16]>/`, gồm `files/` 0700, `khoa` và `tai_khoan.json` 0600 như Python.
Khi chưa có sổ và `TG_PHONE` cùng `tdlib/files` có sẵn, gateway giữ nguyên đường phiên, khóa và dữ liệu cũ;
HTTP hoạt động trong lúc đăng nhập nền. Khi thêm tài khoản thứ hai, mục cũ được lưu với `legacy_session: true`
để lần khởi động sau vẫn mở đúng đường cũ. Nếu có sổ, mọi mục `online` được khôi phục ở nền, không gửi lại OTP.
Tệp nhập `login_code.txt` và `login_password.txt` chỉ dành cho phiên một tài khoản cũ.

Khác Python có chủ đích: `DELETE /ui/api/accounts/{phone}` đóng cuộc gọi và chờ TDLib xác nhận `Closed`,
rồi chuyển dữ liệu sang `tdlib/.da-xoa-<timestamp>-<hash>` và bỏ mục trong sổ. Dữ liệu không bị xóa.
Với phiên cũ ở gốc `tdlib`, chỉ các thành phần `files`, `khoa`, `tai_khoan.json` được chuyển, tránh kéo theo
phiên của tài khoản khác. Có thể khôi phục thủ công từ thư mục lưu giữ khi gateway đã dừng.
Mỗi tài khoản có ChatService, CallManager và NTgCalls riêng; như Python, cuộc gọi đi thứ hai tới cùng
người nhận đang có cuộc gọi bị từ chối 409 `CALL_BUSY`.

Kiểm không dùng Telegram thật: `TGGW_BUILD_DIR=/opt/tggw-build-h2 ./build.sh test`,
`TGGW_BUILD_DIR=/opt/tggw-build-h2 ./build.sh build`, sau đó
`TGGW_BUILD_DIR=/opt/tggw-build-h2 bash tests/contract/thu_hoan2.sh`.
Hộp đen dựng libtdjson rỗng, dùng container `tggw-h2-thu` tại cổng 3399 và tự dừng khi kết thúc.
