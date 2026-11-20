# Kết quả module cuộc gọi

- `CallManager` hiện thực `CallControl`: gọi đi, nhận và nghe cuộc gọi đến, cúp máy, danh sách và sự kiện; TDLib chỉ qua `TdLink`, media qua NTgCalls 3.0.7.
- Dựng media khi Ready, chuyển báo hiệu base64 hai chiều, dọn media và tệp khi kết thúc; không nghe lại cuộc gọi đã xử lý.
- P2P trực tiếp mặc định tắt; thương lượng phiên bản như Python và kết thúc có lý do nếu không có bản chung.
- Đổi mic/camera, lưu cấu hình mặc định, nhận tiếng qua `microphone` của PLAYBACK ở các chế độ `external`, `file`, `shell`, `push`. Push r1_speaker/dahua_talk dùng lệnh cố định, không lộ khóa trong thông tin cuộc gọi.
- `/say` nhận `audio_b64` và khôi phục mic sau khi phát. JSON thông tin giữ các khóa Python; chưa có bộ tạo JPEG, đo CPU/nhịp media hoặc tổng hợp giọng từ văn bản (501).
- Cast kiểm tra địa chỉ và trả 501 kèm TODO; chưa nối tầng REST hay thử cuộc gọi Telegram thật.
- `TGGW_BUILD_DIR=/opt/tggw-build-calls ./build.sh test`: 30 đạt, 0 lỗi.
- `TGGW_BUILD_DIR=/opt/tggw-build-calls ./build.sh build`: dựng release thành công; 2 cảnh báo mã chưa dùng trong `src/td.rs`.
- Commit mã cục bộ: `d281961` (module), `a85c1a5` (khôi phục mic), `a4cdbbf` (mã cuộc gọi TDLib), `b012bec` (nguồn mặc định và loa khi gọi đi).
