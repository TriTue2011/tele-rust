"""TC-13: exercise both production pages in headless Chrome with a local API fixture."""

import base64
import io
import json
import os
from pathlib import Path
import subprocess
import re
from tempfile import TemporaryDirectory
import threading
import time
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import urlopen

import websocket
from PIL import Image


WEB = Path(__file__).resolve().parents[2] / "web"
CSP = re.search(r'header::CONTENT_SECURITY_POLICY,"([^"]+)"', (WEB.parent / "src/web.rs").read_text())[1]
USER = "tai-khoan-gia"
PASSWORD = "mat-khau-gia-de-thu"
TOKEN = "token-gia-de-thu"
SESSION = "tggw_session=phien-gia-de-thu"
image = io.BytesIO()
Image.new("RGB", (16, 16), "green").save(image, format="JPEG")
JPEG = image.getvalue()
audio = io.BytesIO()
with wave.open(audio, "wb") as wav:
    wav.setnchannels(1)
    wav.setsampwidth(2)
    wav.setframerate(48_000)
    wav.writeframes(b"\0\0" * 48_000)
WAV = audio.getvalue()


class Fixture(BaseHTTPRequestHandler):
    configured = False
    password = PASSWORD
    online = True

    def log_message(self, *_):
        pass

    def reply(self, data, kind="application/json", cookie=None, status=200):
        body = json.dumps(data).encode() if kind == "application/json" else data
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        if kind.startswith("text/html"):
            self.send_header("Content-Security-Policy", CSP)
        elif kind.startswith(("image/", "audio/", "multipart/")):
            self.send_header("Content-Security-Policy", "default-src 'none'; sandbox")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        if cookie:
            self.send_header("Set-Cookie", cookie)
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Navigating away cancels the chat page's pending feed request.

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path in ("/", "/ui/app.html", "/ui/index.html"):
            self.reply((WEB / ("index.html" if path.endswith("index.html") else "app.html")).read_bytes(), "text/html; charset=utf-8")
        elif path == "/ui/qrcode.min.js":
            self.reply((WEB / "qrcode.min.js").read_bytes(), "text/javascript; charset=utf-8")
        elif path == "/security-media.html":
            self.reply(b'<img id="snapshot" src="/security/snapshot"><img id="view" src="/security/view"><audio id="audio" preload="auto" src="/security/audio"></audio>', "text/html; charset=utf-8")
        elif path == "/security/snapshot":
            self.reply(JPEG, "image/jpeg")
        elif path == "/security/audio":
            self.reply(WAV, "audio/wav")
        elif path == "/security/view":
            self.reply(b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: " + str(len(JPEG)).encode() + b"\r\n\r\n" + JPEG + b"\r\n--frame--\r\n", "multipart/x-mixed-replace; boundary=frame")
        elif path == "/ui/api/auth/status":
            signed_in = self.configured and self.headers.get("Cookie") == SESSION
            self.reply({"da_thiet_lap": self.configured, "dang_nhap": signed_in, "nguoi_dung": USER if signed_in else None})
        elif path == "/health":
            self.reply({"version": "test", "status": "ok", "accounts_online": 1, "accounts_total": 1, "mock_mode": True})
        elif path == "/favicon.ico":
            self.reply(b"", "image/x-icon")
        elif self.headers.get("Cookie") != SESSION:
            self.reply({"error": {"message": "Phiên đăng nhập đã hết hạn."}}, status=401)
        else:
            assert "Authorization" not in self.headers
            if path == "/api/me":
                if not self.online:
                    self.reply({"error": {"code": "ACCOUNT_OFFLINE", "message": "Chưa có tài khoản Telegram trực tuyến."}}, status=503)
                else:
                    self.reply({"id": 1, "name": "Tài khoản thử", "phone": ""})
            else:
                if path == "/api/feed":
                    time.sleep(0.05)
                self.reply({"tai_khoan": [], "items": [], "chats": [], "calls": [], "cursor": 0})

    def do_POST(self):
        assert self.headers["X-Requested-With"] == "fetch"
        assert "Authorization" not in self.headers
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/ui/api/auth/logout-all":
            assert self.headers.get("Cookie") == SESSION
            self.reply({"ok": True}, cookie=SESSION + "; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
            return
        if self.path == "/ui/api/auth/password":
            assert self.headers.get("Cookie") == SESSION
            assert body == {"cu": self.password, "moi": "mat-khau-moi-trinh-duyet"}
            Fixture.password = body["moi"]
            self.reply({"ok": True})
            return
        if self.path == "/ui/api/auth/setup":
            assert not self.configured
            assert body == {"ten": USER, "mat_khau": PASSWORD, "token": TOKEN}
            Fixture.configured = True
        else:
            assert self.path == "/ui/api/auth/login"
            assert self.configured
            assert body == {"ten": USER, "mat_khau": self.password, "ghi_nho": True}
        self.reply({"ok": True}, cookie=SESSION + "; HttpOnly; SameSite=Lax; Path=/; Max-Age=7776000")


def until(check):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if result := check():
            return result
        time.sleep(0.05)
    raise AssertionError("Timed out waiting for Chrome")


class Chrome:
    def __init__(self, profile):
        self.profile = Path(profile)
        self.process = subprocess.Popen([
            os.environ.get("CHROME", "google-chrome"), "--headless", "--no-sandbox",
            "--disable-background-networking", "--no-first-run", "--no-default-browser-check",
            "--remote-debugging-port=0", f"--user-data-dir={profile}", "about:blank",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def __enter__(self):
        port_file = self.profile / "DevToolsActivePort"
        until(port_file.is_file)
        port = port_file.read_text().splitlines()[0]
        with urlopen(f"http://127.0.0.1:{port}/json/list", timeout=5) as response:
            page = next(item for item in json.load(response) if item["type"] == "page")
        self.socket = websocket.create_connection(page["webSocketDebuggerUrl"], timeout=10, suppress_origin=True)
        self.next_id = 0
        self.exceptions = []
        self.requests = []
        self.call("Runtime.enable")
        self.call("Network.enable")
        self.call("Emulation.setDeviceMetricsOverride", width=1280, height=800, deviceScaleFactor=1, mobile=False)
        return self

    def __exit__(self, *_):
        try:
            self.call("Browser.close")
        except websocket.WebSocketConnectionClosedException:
            pass
        finally:
            self.socket.close()
            self.process.wait(timeout=10)
            (self.profile / "DevToolsActivePort").unlink(missing_ok=True)

    def call(self, method, **params):
        self.next_id += 1
        self.socket.send(json.dumps({"id": self.next_id, "method": method, "params": params}))
        while True:
            item = json.loads(self.socket.recv())
            if item.get("method") == "Runtime.exceptionThrown":
                self.exceptions.append(item["params"]["exceptionDetails"].get("text"))
            if item.get("method") == "Runtime.consoleAPICalled" and item["params"]["type"] in ("error", "warning"):
                self.exceptions.append(item["params"]["type"])
            if item.get("method") == "Network.requestWillBeSent":
                self.requests.append(item["params"]["request"]["url"])
            if item.get("id") == self.next_id:
                assert "error" not in item, method
                return item.get("result")

    def evaluate(self, expression):
        result = self.call("Runtime.evaluate", expression=expression, returnByValue=True)
        assert "exceptionDetails" not in result
        return result["result"].get("value")

    def open(self, url, selector):
        self.call("Page.navigate", url=url)
        self.visible(selector)

    def visible(self, selector):
        until(lambda: self.evaluate(f"Boolean(document.querySelector({json.dumps(selector)})?.getClientRects().length)"))
        assert not self.exceptions, self.exceptions

    def fill(self, selector, value):
        self.evaluate(f"document.querySelector({json.dumps(selector)}).focus()")
        self.call("Input.insertText", text=value)

    def enter(self):
        self.call("Input.dispatchKeyEvent", type="keyDown", key="Enter", code="Enter", text="\r", windowsVirtualKeyCode=13)
        self.call("Input.dispatchKeyEvent", type="keyUp", key="Enter", code="Enter", windowsVirtualKeyCode=13)

    def click(self, selector):
        self.evaluate(f"document.querySelector({json.dumps(selector)}).scrollIntoView({{block:'center'}})")
        point = self.evaluate(f"(() => {{ const r = document.querySelector({json.dumps(selector)}).getBoundingClientRect(); return {{x:r.x+r.width/2,y:r.y+r.height/2}}; }})()")
        self.call("Input.dispatchMouseEvent", type="mousePressed", button="left", clickCount=1, **point)
        self.call("Input.dispatchMouseEvent", type="mouseReleased", button="left", clickCount=1, **point)

    def screenshot(self, name):
        if os.environ.get("UI_AUTH_SCREENSHOTS"):
            image = self.call("Page.captureScreenshot", format="png")["data"]
            (Path.cwd() / f".h2-auth-{name}.png").write_bytes(base64.b64decode(image))

    def empty_storage(self):
        assert self.evaluate("localStorage.length + sessionStorage.length") == 0


def main():
    with ThreadingHTTPServer(("127.0.0.1", 0), Fixture) as fixture, TemporaryDirectory(dir=Path.cwd()) as profile:
        threading.Thread(target=fixture.serve_forever, daemon=True).start()
        base = f"http://127.0.0.1:{fixture.server_port}"
        with Chrome(profile) as chrome:
            chrome.open(base + "/ui/index.html", "#auth-setup")
            until(lambda: chrome.evaluate("typeof window.qrcode === 'function'"))
            assert all(url.startswith((base + "/", "data:", "about:")) for url in chrome.requests), chrome.requests
            chrome.open(base + "/", "#gate-setup")
            for width in (320, 768, 1024, 1440):
                chrome.call("Emulation.setDeviceMetricsOverride", width=width, height=800, deviceScaleFactor=1, mobile=width < 768)
                assert chrome.evaluate("document.documentElement.scrollWidth <= innerWidth")
            chrome.call("Emulation.setDeviceMetricsOverride", width=1280, height=800, deviceScaleFactor=1, mobile=False)
            chrome.screenshot("setup-desktop")
            chrome.fill("#setup-token", TOKEN)
            chrome.fill("#setup-user", USER)
            chrome.fill("#setup-pass", PASSWORD)
            chrome.enter()
            chrome.visible("#app:not(.locked) aside")
            chrome.empty_storage()
        with Chrome(profile) as chrome:
            chrome.open(base + "/ui/index.html", "#auth-active")
            chrome.open(base + "/", "#app:not(.locked) aside")
            chrome.call("Network.clearBrowserCookies")
            chrome.call("Page.navigate", url="about:blank")
            chrome.open(base + "/ui/index.html", "#auth-login")
            assert chrome.evaluate("document.querySelector('#remember').checked && document.querySelector('#auth-setup').hidden")
            chrome.call("Emulation.setDeviceMetricsOverride", width=390, height=844, deviceScaleFactor=1, mobile=True)
            chrome.open(base + "/", "#gate-login")
            assert chrome.evaluate("document.querySelector('#remember').checked && document.querySelector('#gate-setup').hidden")
            assert chrome.evaluate("document.querySelector('#btn-auth-login').getBoundingClientRect().bottom < innerHeight")
            chrome.screenshot("login-mobile")
            accessibility = chrome.call("Accessibility.getFullAXTree")["nodes"]
            assert any(node.get("name", {}).get("value") == "Tên đăng nhập" for node in accessibility)
            chrome.fill("#auth-user", USER)
            chrome.fill("#auth-pass", PASSWORD)
            chrome.enter()
            chrome.visible("#app:not(.locked) aside")
            chrome.empty_storage()
            Fixture.online = False
            chrome.open(base + "/", "#gate-account")
            assert chrome.evaluate("document.querySelector('#gate-login').hidden && document.querySelector('#gate-setup').hidden")
            assert chrome.evaluate("document.querySelector('#gate-account a').getAttribute('href')") == "/ui/index.html"
            Fixture.online = True
            chrome.call("Emulation.setDeviceMetricsOverride", width=1280, height=800, deviceScaleFactor=1, mobile=False)
            chrome.open(base + "/ui/index.html", "#auth-active")
            chrome.fill("#old-pass", PASSWORD)
            chrome.fill("#new-pass", "mat-khau-moi-trinh-duyet")
            chrome.click("#btn-password")
            until(lambda: chrome.evaluate("document.querySelector('#auth-msg').textContent") == "Đã đổi mật khẩu.")
            chrome.click("#btn-logout-all")
            chrome.visible("#auth-login")
            chrome.fill("#auth-user", USER)
            chrome.fill("#auth-pass", Fixture.password)
            chrome.enter()
            chrome.visible("#auth-active")
            chrome.screenshot("management-desktop")
            chrome.empty_storage()
        Fixture.configured = False
        Fixture.password = PASSWORD
        with TemporaryDirectory(dir=Path.cwd()) as other_profile, Chrome(other_profile) as chrome:
            chrome.open(base + "/ui/index.html", "#auth-setup")
            chrome.fill("#setup-token", TOKEN)
            chrome.fill("#setup-user", USER)
            chrome.fill("#setup-pass", PASSWORD)
            chrome.enter()
            chrome.visible("#auth-active")
            chrome.open(base + "/", "#app:not(.locked) aside")
            chrome.empty_storage()
        with TemporaryDirectory(dir=Path.cwd()) as media_profile, Chrome(media_profile) as chrome:
            chrome.open(base + "/security-media.html", "#snapshot")
            until(lambda: chrome.evaluate("document.querySelector('#snapshot').naturalWidth === 16 && document.querySelector('#view').naturalWidth === 16 && document.querySelector('#audio').readyState >= 1"))
            assert not chrome.exceptions, chrome.exceptions
        fixture.shutdown()
    print("CAO-01 Chrome media PASS — JPEG, MJPEG and WAV decode with CSP sandbox, nosniff and CORP")
    print("TC-13 PASS — both setup forms, profile restart, cookie deletion, password login/change, logout-all, offline guidance, mobile layout and empty storage")


if __name__ == "__main__":
    main()
