# tele-rust

Gateway **Telegram bằng Rust** (tài khoản người dùng: tin nhắn + gọi thoại/video P2P) cho Home Assistant, thay cho bản
Python (`Telegram-calling/telegram-gateway`). TDLib 1.8.69 (`libtdjson`, gọi qua JSON, mở lúc chạy) + NTgCalls 3.0.7 (liên kết tĩnh).

> **Trạng thái: đang làm (cột mốc 0 xong).** `tggw version` → TDLib 1.8.69; `tggw protocol` → NTgCalls 3.0.7, tgcalls 8/9/12/13.
> Đang viết: client TDLib + đăng nhập, REST đúng hợp đồng bản Python, quản lý cuộc gọi (`src/contract.rs` là hợp đồng giữa các phần).

Dựng: đặt `TGGW_BUILD_HOST` thành SSH alias của máy dựng, hoặc ghi alias đó vào `.tggw-build-host`
ở gốc worktree, rồi chạy `./build.sh build` (mặc định `builder` nếu chưa cấu hình).
Tệp `.tggw-build-host` được Git bỏ qua và không gửi trong gói nguồn; biến môi trường được ưu tiên.
Lệnh dùng container `rust:1.90` + g++/zlib; tắt LTO vì thư viện C++ của NTgCalls không có bitcode.
Lưu ý giấy phép: NTgCalls là LGPL-3.0 liên kết tĩnh — phát hành nhị phân cần xem lại điều khoản liên kết lại. Giấy phép mã của kho này: chưa chọn.

## Nhiều tài khoản và đăng nhập trên web

`tggw serve` phục vụ giao diện trò chuyện tại `/` và `/ui/app.html`, quản lý tài khoản tại `/ui/index.html`.

### Đăng nhập web

Lần đầu mở trang, chọn **Thiết lập lần đầu**: dán token khởi tạo đang cấu hình cho gateway một lần, rồi tạo tên đăng nhập
và mật khẩu (tối thiểu 10 ký tự, không được trùng tên đăng nhập). Token không được lưu trong trình duyệt. Sau đó đăng nhập
bằng tên và mật khẩu; **Ghi nhớ trên trình duyệt này** bật sẵn để phiên được giữ tối đa 90 ngày và gia hạn khi dùng.
Bỏ chọn mục này để dùng cookie phiên, trình duyệt sẽ bỏ cookie khi đóng. Cookie dùng `HttpOnly`, `SameSite=Lax`, `Path=/`
và có `Secure` trên HTTPS. Trình duyệt gọi API bằng cookie cùng nguồn và kiểm tra CSRF cho yêu cầu ghi.
Xoá cookie thì đăng nhập lại bằng tên và mật khẩu, không cần dán token. Trong trang quản lý, **Đăng xuất** thu hồi
phiên của trình duyệt hiện tại; **Đăng xuất mọi nơi** thu hồi mọi phiên ngay lập tức.

Kho đăng nhập nằm tại `<TGGW_DATA>/ui_auth.json` (mặc định `/data`), tệp quyền `0600`, thư mục dữ liệu quyền `0700`.
Có thể tạo tài khoản web tự động ở lần khởi động đầu bằng cách đặt đồng thời `TG_UI_USER` và `TG_UI_PASSWORD`;
các biến này bị bỏ qua khi kho đăng nhập đã tồn tại. Có thể bật cờ cookie bảo mật tường minh bằng
`TGGW_UI_COOKIE_SECURE=1` để buộc cờ `Secure`, chẳng hạn khi TLS kết thúc ở proxy.

Đổi mật khẩu trong trang quản lý bằng mật khẩu hiện tại. Phiên đang đổi mật khẩu được giữ, các phiên khác hết hiệu lực ngay.
Nếu quên mật khẩu, chạy `tggw passwd` trên máy chủ với cùng thư mục dữ liệu của gateway. Lệnh đọc `TG_UI_PASSWORD`
hoặc hỏi mật khẩu mới trong terminal và ẩn nội dung nhập. Mọi phiên bị thu hồi ngay, kể cả khi gateway đang chạy.

Token gateway vẫn là thông tin xác thực máy-gọi-máy: Home Assistant và các tích hợp hiện có tiếp tục dùng
`Authorization: Bearer …` không thay đổi. Các API `/ui/api/accounts*` cũng chấp nhận Bearer như trước; giao diện web
không giữ hoặc gửi token sau lần thiết lập.

Tệp nhận từ Telegram được cô lập bằng CSP và `nosniff`. Chỉ ảnh raster PNG/JPEG/GIF/WebP, âm thanh và video
hiển thị trực tiếp; các kiểu khác (kể cả HTML, SVG, PDF và nhật ký cuộc gọi) được tải xuống.
Thư viện vẽ QR được nhúng trong gateway, không tải mã từ CDN trên trang đăng nhập.

Chọn tài khoản trong giao diện trò chuyện, hoặc truyền
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

## Thiết bị của máy đang mở trình duyệt (hợp đồng máy chủ)

`POST /api/live/call` và `/api/live/settings` nhận `mic: "browser"`, `speaker: "browser"`.
Mic được lấy từ các khối PCM do trang gửi; speaker browser dùng playback External của NTgCalls,
không phát tới loa HA. Các khóa JSON cũ của cuộc gọi được giữ nguyên. Đây là phần máy chủ để giao diện nối vào.

| Đường | Yêu cầu và phản hồi |
| --- | --- |
| `POST /api/live/audio/in/{call_id}` | Bearer ở header; thân thô PCM s16le 16 kHz mono, không rỗng, số byte chẵn, ≤64 KiB. Thành công `{ok:true,bytes:N}`. Nên gửi 3200 byte mỗi 100 ms. |
| `POST /api/live/media` | Bearer; `{id,action:"audio_out"}` → `{audio_out:{url,content_type:"audio/wav",expires_in}}`. |
| `GET /api/live/audio/out/{call_id}?t=…` | URL tương đối được cấp ở trên, dùng cho `<audio src>`. WAV PCM s16le 48 kHz mono; không cần Bearer. |
| `GET /api/live/settings` | Bearer; thêm `auto` vào các khóa hiện có, nguồn có bí mật vẫn được che như trước. |
| `GET /api/live/devices` | Bearer; `{settings,auto,catalog:{mic:[],speaker:[],camera:[]}}`. |
| `POST /api/live/ha_devices` | Bearer; `{mic:[{id,name}],speaker:[{id,name}],camera:[{id,name}]}` thay toàn bộ danh mục; trả cùng hình dạng GET devices. Nhóm thiếu thành mảng rỗng. |

Mic trả 400 nếu khối PCM sai, 401 nếu thiếu/sai Bearer, 404 khi cuộc gọi không còn, 409 khi chưa active hoặc
không dùng mic browser, 413 khi quá lớn, 429 khi quá nhanh/đầy, 408 nếu thân gửi quá 5 giây.
Hạn mức riêng là 30 POST/giây, 32.000 byte/giây với burst 64 KiB, hàng chờ 64 KiB mỗi cuộc gọi;
tối đa 8 request PCM đang nhận và 8 khối chờ actor. Các POST này không tiêu hạn mức điều khiển 120/phút.
Nguồn NTgCalls chỉ nghe loopback có khóa ngẫu nhiên riêng và luôn bù im lặng khi thiếu dữ liệu.

Token âm thanh là khóa ngẫu nhiên 192 bit, riêng từng cuộc gọi, hạn tối đa 600 giây. Khi luồng hết hạn,
lấy lại descriptor qua POST media rồi gắn URL mới. Token sai/hết hạn trả 403, cuộc gọi hết trả 404,
đủ hai người nghe trả 429. Người nghe hiện tại cũng bị ngắt khi hết hạn hoặc kết thúc cuộc gọi.
Bộ đệm phát có 128 khối tối đa 1920 byte; người nghe tụt hậu bị đóng luồng. Không truyền `GATEWAY_TOKEN` trên query.

`auto` đúng nếu có mic/camera/loa ngoài do cấu hình HA hoặc quy tắc đặt; các nguồn dựng sẵn
`test`, `silence`, `black`, và `browser` không được tính là tự động. Danh mục HA ban đầu rỗng;
mỗi nhóm tối đa 64 mục, ID/URL được lọc và khử trùng, tên tối đa 120 ký tự. Tệp `ha_devices.json`
được ghi nguyên tử với quyền 0600 trong dữ liệu của từng tài khoản. Việc HA đẩy danh mục cần được nối riêng.
GET thiết bị và POST danh mục nhận `?account=…`; POST cũng nhận `account` trong JSON.
Âm thanh vào/ra chọn tài khoản theo mã cuộc gọi duy nhất do gateway sinh.

Camera trình duyệt được hoãn: cấu hình nhận `camera:"browser"`, nhưng bật video với nguồn này trả
501 `BROWSER_VIDEO_UNAVAILABLE` trước khi tạo/đổi media. Chưa có `POST /api/live/video/in/{call_id}`.
Chọn `black` hoặc camera HA cho cuộc gọi video. Âm thanh với nguồn browser đã được kiểm bằng TDLib/media giả;
độ trễ, khử vọng và khả năng phát WAV cần kiểm tiếp với trình duyệt và cuộc gọi Telegram thật.

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
