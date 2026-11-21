#!/usr/bin/env python3
"""Kiểm tra các hình dạng JSON ổn định của REST Python/Rust."""

import json
import re
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


def request(base_url, path, token=None, method="GET", raw=False):
    headers = {}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    req = Request(base_url.rstrip("/") + path, headers=headers, method=method)
    try:
        response = urlopen(req, timeout=10)
    except HTTPError as error:
        response = error
    except URLError as error:
        raise AssertionError(f"Không kết nối được {path}: {error}") from error
    with response:
        status = response.status
        if raw:
            return status, (response.headers, response.read().decode("utf-8"))
        try:
            body = json.loads(response.read())
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise AssertionError(f"{path} không trả JSON hợp lệ") from error
    return status, body


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def object_shape(value, fields, path):
    require(isinstance(value, dict) and set(value) == set(fields),
            f"{path} có tập khóa không đúng")
    for key, types in fields.items():
        require(type(value[key]) in types, f"{path}.{key} sai kiểu")


def call_shape(call):
    """LiveCall.info() và VideoView.info() trong bản Python tham khảo."""
    number = (int, float)
    object_shape(call, {
        "id": (str,), "user_id": (int,), "outgoing": (bool,),
        "video": (bool,), "state": (str,), "emojis": (list,),
        "error": (str,), "seconds": number, "peer_video": (bool,),
        "view": (dict,), "diag": (dict,), "sig": (dict,),
        "cpu_max": number, "mic_muted": (bool,), "cam_off": (bool,),
        "frames_in": (int,), "frame_bytes": (int,), "peak": number,
        "playback": (dict,), "events": (list,),
    }, "calls[]")
    require(all(type(emoji) is str for emoji in call["emojis"]),
            "calls[].emojis[] phải là string")
    object_shape(call["view"], {
        "frames": (int,), "jpeg": (int,), "size": (str,),
        "dropped": (int,), "bad": (int,),
    }, "calls[].view")
    object_shape(call["sig"], {key: (int,) for key in ("tx_n", "tx_b", "rx_n", "rx_b")},
                 "calls[].sig")
    object_shape(call["playback"], {"kind": (str,), "bytes": (int,)}, "calls[].playback")
    for event in call["events"]:
        object_shape(event, {"t": number, "e": (str,)}, "calls[].events[]")


def settings_shape(settings):
    object_shape(settings, {**{key: (str,) for key in
                              ("mic", "mic_format", "camera", "speaker", "playback")},
                            "auto": (bool,)}, "settings")


def devices_shape(devices):
    object_shape(devices, {"settings": (dict,), "auto": (bool,), "catalog": (dict,)}, "devices")
    settings_shape(devices["settings"])
    require(devices["auto"] == devices["settings"]["auto"], "auto không nhất quán")
    object_shape(devices["catalog"], {key: (list,) for key in ("mic", "speaker", "camera")}, "catalog")
    for kind, entries in devices["catalog"].items():
        require(len(entries) <= 64, f"catalog.{kind} vượt 64 mục")
        seen = set()
        for entry in entries:
            object_shape(entry, {"id": (str,), "name": (str,)}, f"catalog.{kind}[]")
            require(entry["id"] not in seen and entry["id"] and entry["name"], "Mục rỗng hoặc trùng")
            seen.add(entry["id"])


def audio_out_shape(response):
    object_shape(response, {"audio_out": (dict,)}, "media")
    descriptor = response["audio_out"]
    object_shape(descriptor, {"url": (str,), "content_type": (str,), "expires_in": (int,)}, "audio_out")
    require(re.fullmatch(r"/api/live/audio/out/[a-f0-9]+\?t=[A-Za-z0-9_-]{32}", descriptor["url"]) is not None,
            "URL âm thanh phải chứa token media 192 bit và mã cuộc gọi")
    require(descriptor["content_type"] == "audio/wav" and 0 <= descriptor["expires_in"] <= 600,
            "Sai định dạng hoặc hạn token âm thanh")


def self_test():
    settings = {"mic": "browser", "mic_format": "auto", "camera": "black",
                "speaker": "browser", "playback": "file", "auto": False}
    devices = {"settings": settings, "auto": False,
               "catalog": {"mic": [{"id": "sensor.mic", "name": "Mic"}], "speaker": [], "camera": []}}
    devices_shape(devices)
    audio = {"audio_out": {"url": "/api/live/audio/out/abcdef123456?t=" + "a" * 32,
                           "content_type": "audio/wav", "expires_in": 599}}
    audio_out_shape(audio)
    rejected = 0
    for validate, invalid in ((settings_shape, {**settings, "auto": 1}),
                              (devices_shape, {**devices, "auto": True}),
                              (audio_out_shape, {"audio_out": {**audio["audio_out"], "url": "/?token=Bearer"}})):
        try:
            validate(invalid)
        except AssertionError:
            rejected += 1
    require(rejected == 3, "Bộ kiểm phải từ chối cả ba hình dạng sai")
    print("Hợp đồng thiết bị/media: 3 hình dạng đúng đạt; 3 hình dạng sai bị từ chối")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    if len(sys.argv) != 3:
        raise SystemExit(f"Cách dùng: {sys.argv[0]} BASE_URL TOKEN")
    base_url, token = sys.argv[1:]

    for path in ("/", "/ui/app.html", "/ui/index.html"):
        status, (headers, html) = request(base_url, path, raw=True)
        require(status == 200 and "<!DOCTYPE html>" in html, f"GET {path} phải trả HTML 200")
        require(headers.get("X-Content-Type-Options") == "nosniff", "Thiếu nosniff")
        require(headers.get("Referrer-Policy") == "no-referrer", "Thiếu no-referrer")
        require("connect-src 'self'" in headers.get("Content-Security-Policy", ""), "Thiếu CSP")
        print(f"GET {path}: HTTP 200, HTML và tiêu đề an toàn đạt")

    status, unauthorized = request(base_url, "/ui/api/accounts", method="POST")
    require(status == 401 and unauthorized["error"]["code"] == "UNAUTHORIZED", "Đăng nhập HTTP thiếu token phải trả 401")
    print("POST /ui/api/accounts: HTTP 401 khi thiếu token")

    status, accounts = request(base_url, "/api/accounts", token)
    require(status == 200, "GET /api/accounts phải trả 200")
    object_shape(accounts, {"tai_khoan": (list,)}, "accounts")
    for account in accounts["tai_khoan"]:
        object_shape(account, {"phone": (str,), "display_name": (str,), "status": (str,)}, "tai_khoan[]")
    print("GET /api/accounts: HTTP 200, danh sách đúng khóa và kiểu")

    status, health = request(base_url, "/health")
    require(status == 200, f"GET /health: HTTP {status}, cần 200")
    require(isinstance(health, dict), "GET /health phải trả object JSON")
    require(set(health) == {
        "status", "version", "uptime_seconds", "accounts_total",
        "accounts_online", "mock_mode",
    }, "GET /health có tập khóa không đúng")
    require(isinstance(health["status"], str), "health.status phải là string")
    require(isinstance(health["version"], str), "health.version phải là string")
    for key in ("uptime_seconds", "accounts_total", "accounts_online"):
        require(type(health[key]) is int, f"health.{key} phải là integer")
    require(type(health["mock_mode"]) is bool, "health.mock_mode phải là boolean")
    print("GET /health: HTTP 200, JSON đúng khóa và kiểu")

    for label, auth in (("Bearer thiếu", None), ("Bearer sai", token + "-sai")):
        status, unauthorized = request(base_url, "/api/accounts", auth)
        require(status == 401, f"{label}: HTTP {status}, cần 401")
        object_shape(unauthorized, {"error": (dict,)}, "Lỗi xác thực")
        object_shape(unauthorized["error"], {"code": (str,), "message": (str,)}, "error")
        require(unauthorized["error"]["code"] == "UNAUTHORIZED", "Sai mã lỗi xác thực")
        print(f"{label}: HTTP 401, JSON đúng khóa và kiểu")

    status, calls = request(base_url, "/api/live/calls", token)
    require(status == 503, f"GET /api/live/calls khi chưa đăng nhập: HTTP {status}, cần 503")
    object_shape(calls, {"error": (dict,)}, "Lỗi tài khoản chưa đăng nhập")
    object_shape(calls["error"], {"code": (str,), "message": (str,)}, "error")
    require(calls["error"]["code"] == "ACCOUNT_OFFLINE",
            "GET /api/live/calls khi chưa đăng nhập phải là ACCOUNT_OFFLINE")
    require(calls["error"]["message"] == "Tài khoản đang mất kết nối tới Telegram.",
            "Thông điệp ACCOUNT_OFFLINE không khớp Python")
    print("GET /api/live/calls: HTTP 503 ACCOUNT_OFFLINE, JSON đúng khóa và kiểu")

    for path, method in (("/api/live/devices", "GET"), ("/api/live/ha_devices", "POST"),
                         ("/api/live/audio/in/abcdef123456", "POST")):
        status, error = request(base_url, path, method=method)
        require(status == 401 and error["error"]["code"] == "UNAUTHORIZED", f"{path} phải cần Bearer")
    status, error = request(base_url, "/api/live/audio/out/abcdef123456?t=wrong")
    require(status == 403 and error["error"]["code"] == "MEDIA_TOKEN", "Luồng âm thanh phải từ chối token sai")
    print("Đường thiết bị/media: thiếu Bearer trả 401; token audio sai trả 403")


if __name__ == "__main__":
    main()
