#!/usr/bin/env python3
"""Phát hành gateway Telegram Rust: TỰ TĂNG phiên bản, gắn thẻ git, tạo GitHub Release.

Dùng (mỗi khi có bản mới trên main):
    GH_TOKEN=<token> python3 deploy/phat-hanh.py            # tăng PATCH: 0.2.0 -> 0.2.1
    GH_TOKEN=<token> python3 deploy/phat-hanh.py minor      # 0.2.1 -> 0.3.0
    GH_TOKEN=<token> python3 deploy/phat-hanh.py --dry-run  # chỉ in phiên bản kế và ghi chú, không làm gì

Các bước (dừng ngay khi một bước hỏng, không để dở nửa chừng ở bước đẩy):
  1. Phải ở nhánh main, cây sạch, và main KHÔNG tụt sau origin/main.
  2. Phiên bản kế = thẻ vX.Y.Z cao nhất (lấy cả từ remote) tăng theo patch/minor/major; chưa có thẻ nào thì
     lấy phiên bản trong Cargo.toml tăng minor (lần đầu: 0.1.0 -> 0.2.0).
  3. `./build.sh test` PHẢI mã thoát 0 (build trên máy dựng .200), nếu không thì không phát hành.
  4. Sửa `version` trong Cargo.toml, commit «release: vX.Y.Z», thẻ có chú thích.
  5. Đẩy main + thẻ, rồi tạo GitHub Release với ghi chú = các commit từ thẻ trước (bỏ commit «release:»).

Bản phát hành là MÃ NGUỒN (thẻ + ghi chú), KHÔNG đính kèm nhị phân: NTgCalls được liên kết tĩnh và là LGPL-3.0,
phát hành nhị phân công khai thì phải cung cấp thêm điều kiện liên kết lại. Dựng bằng `./build.sh build`, áp lên máy
chạy bằng `deploy/ap-ban-sua-tele-rust.sh`; `tggw version` và `/health` in đúng phiên bản của thẻ.

Bí mật: token chỉ đọc từ biến môi trường GH_TOKEN, truyền cho git qua biến GIT_CONFIG_* (không lên dòng lệnh,
không ghi tệp), và bị che trong mọi dòng in ra.
"""
import base64
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")


def die(msg, code=1):
    print("DỪNG:", msg)
    sys.exit(code)


TOKEN = os.environ.get("GH_TOKEN", "")


def che(text):
    return text.replace(TOKEN, "***") if TOKEN else text


def git(*args, auth=False, check=True, capture=True):
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
    if auth:
        basic = base64.b64encode(("x-access-token:" + TOKEN).encode()).decode()
        env.update(GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0="http.extraheader",
                   GIT_CONFIG_VALUE_0="Authorization: Basic " + basic)
    r = subprocess.run(["git", "-C", REPO] + list(args), capture_output=capture, text=True, env=env)
    out = che((r.stdout or "") + (r.stderr or "")).strip()
    if check and r.returncode:
        die("git %s: %s" % (" ".join(args[:2]), out))
    return out


def slug():
    url = git("remote", "get-url", "origin")
    m = re.search(r"github\.com[:/]([^/]+/[^/.]+?)(?:\.git)?$", url)
    if not m:
        die("không nhận ra repo GitHub từ remote origin")
    return m.group(1)


def api(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request("https://api.github.com" + path, data=data, method=method, headers={
        "Authorization": "Bearer " + TOKEN, "Accept": "application/vnd.github+json",
        "User-Agent": "tele-rust-phat-hanh", "Content-Type": "application/json"})
    try:
        return json.loads(urllib.request.urlopen(req, timeout=30).read() or b"{}")
    except urllib.error.HTTPError as e:
        die("GitHub API %s %s: HTTP %s %s" % (method, path, e.code, che(e.read().decode()[:300])))


def cargo_version():
    m = re.search(r'(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"', open(os.path.join(REPO, "Cargo.toml"), encoding="utf-8").read())
    if not m:
        die("không đọc được version trong Cargo.toml")
    return m.group(1)


def tags():
    found = set(t for t in git("tag", "--list", "v*").split() if TAG_RE.match(t))
    for line in git("ls-remote", "--tags", "origin", auth=True, check=False).splitlines():
        m = re.search(r"refs/tags/(v\d+\.\d+\.\d+)$", line)
        if m:
            found.add(m.group(1))
    return sorted(found, key=lambda t: tuple(map(int, TAG_RE.match(t).groups())))


def next_version(muc, cac_the):
    if cac_the:
        a, b, c = map(int, TAG_RE.match(cac_the[-1]).groups())
    else:
        a, b, c = map(int, cargo_version().split("."))
        muc = "minor" if muc == "patch" else muc  # lần phát hành đầu: 0.1.0 -> 0.2.0
    if muc == "major":
        return "%d.0.0" % (a + 1)
    if muc == "minor":
        return "%d.%d.0" % (a, b + 1)
    return "%d.%d.%d" % (a, b, c + 1)


def ghi_chu(truoc, ver):
    pham_vi = "%s..HEAD" % truoc if truoc else "HEAD"
    dong = [l for l in git("log", pham_vi, "--no-merges", "--format=%s").splitlines() if l and not l.startswith("release:")]
    thay_doi = "\n".join("- " + l for l in dong) or "- (không có commit mới ngoài phát hành)"
    return ("## Thay đổi từ %s\n\n%s\n\n## Dựng và áp\n\n"
            "- Dựng/kiểm trên máy dựng: `./build.sh test` rồi `./build.sh build`.\n"
            "- Áp lên máy chạy: `deploy/ap-ban-sua-tele-rust.sh` (từ chối khi đang có cuộc gọi, có đường lui về Python).\n"
            "- `tggw version` và `/health` in `%s`.\n\n"
            "Bản phát hành là mã nguồn; không kèm nhị phân vì NTgCalls liên kết tĩnh (LGPL-3.0).\n") % (
        truoc or "đầu tiên", thay_doi, ver)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    dry = "--dry-run" in sys.argv
    muc = args[0] if args else "patch"
    if muc not in ("patch", "minor", "major"):
        die("tham số phải là patch, minor hoặc major")
    if not TOKEN and not dry:
        die("thiếu biến môi trường GH_TOKEN")
    if git("branch", "--show-current") != "main":
        die("phải đứng ở nhánh main")
    if git("status", "--porcelain"):
        die("cây làm việc chưa sạch, hãy commit hoặc dọn trước")
    cac_the = tags()
    ver = next_version(muc, cac_the)
    truoc = cac_the[-1] if cac_the else ""
    print("phiên bản hiện tại: %s | thẻ gần nhất: %s | phiên bản kế: %s" % (cargo_version(), truoc or "(chưa có)", ver))
    print(ghi_chu(truoc, ver))
    if dry:
        print("(--dry-run: không làm gì thêm)")
        return
    if "v" + ver in cac_the:
        die("thẻ v%s đã tồn tại" % ver)
    # main không được tụt sau origin/main
    git("fetch", "origin", "main", auth=True)
    if git("rev-list", "--count", "HEAD..origin/main") != "0":
        die("main cục bộ tụt sau origin/main, hãy kéo về trước")
    if os.environ.get("PHAT_HANH_SKIP_BUILD_TEST"):
        # auto-release.yml chạy trong GitHub Actions: build.sh test cần SSH tới máy dựng ha200 (mạng LAN
        # riêng, cloud runner không với tới), bỏ qua VÌ ci.yml đã tự kiểm cargo test trên cloud runner cho
        # ĐÚNG commit này rồi (workflow riêng, cùng push). Không bỏ qua khi chạy tay/trên máy có ha200.
        print("== bỏ qua ./build.sh test (PHAT_HANH_SKIP_BUILD_TEST=1, ci.yml đã kiểm trên cloud runner)")
    else:
        print("== chạy ./build.sh test (phải mã thoát 0)")
        r = subprocess.run(["./build.sh", "test"], cwd=REPO)
        if r.returncode:
            die("test không đạt (mã %d), không phát hành" % r.returncode)
    # sửa version, commit, thẻ
    path = os.path.join(REPO, "Cargo.toml")
    txt = open(path, encoding="utf-8").read()
    moi = re.sub(r'(?m)^version\s*=\s*"\d+\.\d+\.\d+"', 'version = "%s"' % ver, txt, count=1)
    open(path, "w", encoding="utf-8").write(moi)
    # Add-on HA khai cùng số phiên bản: tests/contract/kiem_addon_ci.py bắt buộc addon/tele-rs/config.yaml == Cargo.toml (CI job add-on đỏ nếu lệch).
    apath = os.path.join(REPO, "addon", "tele-rs", "config.yaml")
    atxt = open(apath, encoding="utf-8").read()
    amoi = re.sub(r'(?m)^version:\s*"\d+\.\d+\.\d+"', 'version: "%s"' % ver, atxt, count=1)
    if amoi == atxt and 'version: "%s"' % ver not in atxt:
        die("không cập nhật được version trong addon/tele-rs/config.yaml")
    open(apath, "w", encoding="utf-8").write(amoi)
    # Số phiên bản còn xuất hiện ở README và tài liệu (tests/contract/kiem_addon_ci.py và readme-matches-code.test.cjs bắt README nhắc v<phiên bản hiện tại>):
    # đổi CÙNG LÚC, nếu không bản phát hành dừng giữa chừng (0.7.3 đã dừng đúng chỗ này). CHANGELOG KHÔNG đổi hàng loạt (lịch sử) — mục mới viết tay.
    old = re.search(r'(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"', txt).group(1)
    for rel in ("README.md", "docs/BAO-MAT.md", "docs/TRIEN-KHAI.md", "docs/API.md", "docs/TINH-NANG.md", "docs/THAM-CHIEU.md"):
        dpath = os.path.join(REPO, rel)
        if os.path.isfile(dpath):
            dtxt = open(dpath, encoding="utf-8").read()
            open(dpath, "w", encoding="utf-8").write(dtxt.replace("v" + old, "v" + ver).replace(":" + old, ":" + ver))
    k = subprocess.run([sys.executable, "tests/contract/kiem_addon_ci.py"], cwd=REPO, capture_output=True, text=True)
    if k.returncode:
        die("kiểm add-on không đạt sau khi tăng phiên bản: " + (k.stdout + k.stderr)[-300:])
    trailer = os.environ.get("PHAT_HANH_TRAILER", "").strip()
    msg = ("release: v%s\n\nLớp việc: đánh dấu một bản phát hành của gateway Tele Rust.\n"
           "Cùng lớp: phiên bản đổi ở Cargo.toml và addon/tele-rs/config.yaml cùng lúc (`grep -n '^version' Cargo.toml addon/tele-rs/config.yaml`), `tggw version` và `/health` đọc từ đó.\n"
           "Nguyên tắc: phiên bản do deploy/phat-hanh.py tăng sau khi `./build.sh test` đạt, không sửa tay.\n"
           % ver) + (("\n" + trailer + "\n") if trailer else "")
    git("add", "Cargo.toml", "addon/tele-rs/config.yaml", "README.md", "docs")
    git("commit", "-q", "-m", msg)
    git("tag", "-a", "v" + ver, "-m", "Tele Rust v" + ver)
    print("== đẩy main + thẻ v" + ver)
    git("push", "origin", "main:main", "v" + ver, auth=True)
    sha = git("rev-parse", "HEAD")
    rel = api("POST", "/repos/%s/releases" % slug(), {
        "tag_name": "v" + ver, "target_commitish": sha, "name": "v" + ver,
        "body": ghi_chu(truoc, ver), "draft": False, "prerelease": False})
    print("XONG: v%s | %s" % (ver, rel.get("html_url")))


if __name__ == "__main__":
    main()
