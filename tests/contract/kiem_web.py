#!/usr/bin/env python3
"""Quét dữ liệu mẫu trong HTML và kiểm cú pháp JavaScript nhúng, không truy cập mạng."""
from html.parser import HTMLParser
from pathlib import Path
import ipaddress
import re
import subprocess


class Scripts(HTMLParser):
    def __init__(self):
        super().__init__()
        self.inline = False
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if tag == "script":
            self.inline = "src" not in dict(attrs)

    def handle_endtag(self, tag):
        if tag == "script":
            self.inline = False

    def handle_data(self, data):
        if self.inline:
            self.parts.append(data)


for path in sorted(Path("web").glob("*.html")):
    html = path.read_text()
    phones = re.findall(r"\+\d{9,15}", html)
    assert all(phone == "+84900000000" for phone in phones), f"{path}: số ngoài danh sách giả"
    ips = re.findall(r"\b(?:\d{1,3}\.){3}\d{1,3}\b", html)
    for ip in ips:
        address = ipaddress.ip_address(ip)
        assert address in ipaddress.ip_network("192.0.2.0/24"), f"{path}: IP ngoài dải tài liệu"
    assert not re.search(r"(?:Bearer\s+[A-Za-z0-9_-]{16,}|tg://login\?token=[A-Za-z0-9_-]+|[a-f0-9]{32,})", html), f"{path}: giá trị giống token nhúng"
    parser = Scripts()
    parser.feed(html)
    result = subprocess.run(["node", "--check"], input="\n".join(parser.parts), text=True, capture_output=True)
    assert result.returncode == 0, result.stderr
    print(f"{path}: JavaScript đạt; 0 IP LAN, 0 số thật, 0 token nhúng; {len(phones)} số giả được phép")
