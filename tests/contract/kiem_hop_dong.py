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
    require(status == 503, f"GET /api/live/calls khi chưa đăng nhập: HTTP {status}, cần 503")
    object_shape(calls, {"error": (dict,)}, "Lỗi tài khoản chưa đăng nhập")
    object_shape(calls["error"], {"code": (str,), "message": (str,)}, "error")
    require(calls["error"]["code"] == "ACCOUNT_OFFLINE",
            "GET /api/live/calls khi chưa đăng nhập phải là ACCOUNT_OFFLINE")
    require(calls["error"]["message"] == "Tài khoản đang mất kết nối tới Telegram.",
            "Thông điệp ACCOUNT_OFFLINE không khớp Python")
    print("GET /api/live/calls: HTTP 503 ACCOUNT_OFFLINE, JSON đúng khóa và kiểu")


if __name__ == "__main__":
    main()
