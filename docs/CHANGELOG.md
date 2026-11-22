# Nhật ký thay đổi (Changelog)

Định dạng tham khảo [Keep a Changelog](https://keepachangelog.com/vi/1.1.0/).

## [Unreleased]

### Đã thêm
- `Dockerfile` đa tầng tự đủ (`rust:1.90-slim-bookworm` + wheel `tdjson==1.8.69` + `debian:bookworm-slim` kèm `ffmpeg`, tuỳ chọn `edge-tts`, `/licenses` chứa `LICENSE` + `NOTICE.md`, chạy non-root UID `10001`).
- `docker/entrypoint.sh` ánh xạ toàn bộ tuỳ chọn `/data/options.json` sang biến môi trường và tách `/data/state` khi chạy dưới Home Assistant Supervisor.
- `.dockerignore`, `LICENSE` (MIT), `NOTICE.md` (giấy phép NTgCalls LGPL-3.0 kèm hướng dẫn liên kết lại + TDLib BSL-1.0 + qrcode-generator MIT).
- Bộ workflow GitHub Actions tại `.github/workflows/`: `ci.yml`, `image.yml` (`linux/amd64` đẩy `ghcr.io/tritue2011/tele-rust` qua `GITHUB_TOKEN`), `release.yml` (tạo GitHub Release kèm tarball mã nguồn, không kèm nhị phân trần) và `addon.yml`.
- Add-on Home Assistant tại `repository.yaml` và `addon/tele-rs/` (`config.yaml`, `README.md`, `DOCS.md`, `icon.png`, `logo.png`).
- Tài liệu chi tiết tại `README.md` và `docs/` (`API.md`, `TRIEN-KHAI.md`, `BAO-MAT.md`, `CHANGELOG.md`).
- Bộ kiểm tự động: `tests/readme-matches-code.test.cjs` (đối chiếu biến môi trường & route REST giữa `README.md` và mã nguồn) và `tests/contract/kiem_addon_ci.py` (kiểm cấu trúc add-on, Dockerfile và workflow).

### Đã xoá / Dọn kho
- Xoá tệp nội bộ `HOAN-TAT.md.SAI-212357` lọt vào kho công khai và bổ sung quy tắc bỏ qua tệp nội bộ vào `.gitignore`.

## [0.7.1] — Phiên bản hiện tại trong cây mã

> **Ghi chú lịch sử thẻ `v0.2.0` → `v0.7.0`:** Trong môi trường hộp cát biệt lập không truy cập được kho `.git` gốc (`gitdir` trỏ ra ngoài hộp cát), không thể đọc trực tiếp danh sách commit/tag lịch sử từ `git log`/`git tag`. Phần tổng hợp dưới đây phản ánh chính xác các năng lực đang có trong mã nguồn tại mốc `v0.7.1`; khi ở môi trường có đầy đủ `.git`, có thể bổ sung chi tiết từng mốc `v0.2.0` → `v0.7.1` bằng lệnh `git tag -l -n9` và `git log --oneline`.

### Năng lực hiện có ở v0.7.1
- **Xác thực web & đa tài khoản:** Cookie `HttpOnly` + PBKDF2-SHA256 (`210.000` vòng), CSRF, đặt lại mật khẩu qua `tggw passwd`, đăng nhập nhiều tài khoản qua OTP/2FA/QR và lưu trữ phiên xoá an toàn (`tdlib/.da-xoa-*`).
- **Trò chuyện & định dạng:** Danh sách hội thoại kiểu macOS (huy hiệu đếm đầy đủ, trạng thái cuộc gọi đi/đến/nhỡ/từ chối, đồng bộ ✓✓ qua `chat_read_outbox`), tin hệ thống tiếng Việt, soạn thảo WYSIWYG UTF-16 `entities` (`B`/`I`/`U`/`S`/`</>`/`pre`/`spoiler`/`blockquote`).
- **Thông tin hội thoại & Danh bạ:** Ngăn Thông tin hội thoại (thông báo riêng, tự xoá tin đồng bộ Telegram, ghim/lưu trữ, thành viên, liên kết mời cho admin, xoá lịch sử/xoá chat/rời nhóm với `confirm: true`), quản lý danh bạ & người bị chặn sắp xếp tiếng Việt ICU.
- **Lưu trữ & Cài đặt:** Chính sách lưu trữ `local`/`stream`, dọn tự động định kỳ giữ ngoại lệ «Mãi mãi», xoá thủ công bỏ qua «Mãi mãi», cấu hình giao diện 10 chủ đề/phông/cỡ chữ đồng bộ popover `Aa`, cập nhật hồ sơ có xác nhận.
- **Cuộc gọi P2P & Tích hợp HA:** NTgCalls `3.0.7` + TDLib `1.8.69`, mic/loa trình duyệt, TTS `edge-tts`, danh mục thiết bị HA (`/api/live/ha_devices`), spool sự kiện và webhook có bộ lọc DNS chống SSRF.
