#!/usr/bin/env python3
"""Kiểm tra các hình dạng JSON ổn định của REST Python/Rust."""

import json
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


def request(base_url, path, token=None):
    headers = {}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    req = Request(base_url.rstrip("/") + path, headers=headers)
    try:
        response = urlopen(req, timeout=10)
    except HTTPError as error:
        response = error
    except URLError as error:
        raise AssertionError(f"Không kết nối được {path}: {error}") from error
    with response:
        status = response.status
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


def main():
    if len(sys.argv) != 3:
        raise SystemExit(f"Cách dùng: {sys.argv[0]} BASE_URL TOKEN")
    base_url, token = sys.argv[1:]

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
    require(status == 200, f"GET /api/live/calls: HTTP {status}, cần 200")
    require(isinstance(calls, dict) and set(calls) == {"calls"},
            "GET /api/live/calls phải có duy nhất khóa calls")
    require(isinstance(calls["calls"], list), "calls phải là array")
    for call in calls["calls"]:
        call_shape(call)
    print("GET /api/live/calls: HTTP 200, JSON đúng khóa và kiểu")
    if not calls["calls"]:
        print("Danh sách calls rỗng: chưa kiểm được hình dạng cuộc gọi thực tế")


if __name__ == "__main__":
    main()
