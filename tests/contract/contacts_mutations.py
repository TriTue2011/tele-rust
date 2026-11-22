"""Đối chứng TC-1/TC-5 trên bản sao; không sửa mã đang làm việc."""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[2]


def fingerprint():
    digest = hashlib.sha256()
    files = [ROOT / name for name in ("Cargo.toml", "build.sh")]
    for directory in ("src", "web", "tests", "docker"):
        files.extend(path for path in (ROOT / directory).rglob("*")
                     if path.is_file() and "__pycache__" not in path.parts)
    for path in sorted(files):
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
    return digest.hexdigest()


def replace_once(path, before, after):
    text = path.read_text()
    assert text.count(before) == 1, f"Điểm đột biến không duy nhất: {path.name}"
    path.write_text(text.replace(before, after, 1))


def main():
    source = fingerprint()
    with TemporaryDirectory(prefix=".h2-contacts-mutant-", dir=ROOT) as directory:
        snapshot = Path(directory)
        for name in ("src", "web", "tests", "docker"):
            shutil.copytree(ROOT / name, snapshot / name, copy_function=shutil.copy,
                            ignore=shutil.ignore_patterns("__pycache__"))
        for name in ("Cargo.toml", "build.sh"):
            shutil.copy2(ROOT / name, snapshot / name)
        target = snapshot / "src/chat/contacts.rs"
        replace_once(target, "b.2.cmp(&a.2)", "a.2.cmp(&b.2)")
        replace_once(target, 'let phone = phone(text(&body["phone"]))?;',
                     'let phone = phone(text(&body["phone"]))?;\n'
                     '                log::debug!("{phone}");')
        replace_once(snapshot / "build.sh", 'ARGS="cargo test --release 2>&1"',
                     'ARGS="cargo test --release --lib chat::tests::contacts:: 2>&1"')
        print("RUN TC-1 (đảo thứ tự) + TC-5 (ghi số vào log)", flush=True)
        result = subprocess.run(["./build.sh", "test"], cwd=snapshot,
                                env={**os.environ, "TGGW_BUILD_DIR": "/opt/tggw-build-danhba"},
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        print(result.stdout, end="", flush=True)
        assert fingerprint() == source, "Mã gốc thay đổi trong khi kiểm đột biến"
        assert result.returncode == 101, f"Mong mã 101, nhận {result.returncode}"
        for name in ("contacts_last_seen_orders_online_timestamps_and_unknown",
                     "contacts_phone_is_not_logged_or_echoed_by_tdlib_errors"):
            assert f"test chat::tests::contacts::{name} ... FAILED" in result.stdout, name
        assert "4 passed; 2 failed;" in result.stdout, "Không đúng hai ca cần thất bại"
        assert "log chứa số điện thoại" in result.stdout, "TC-5 chưa bắt được số trong log"
        # Khôi phục mã và biên dịch lại trước khi trả thư mục dựng cho kiểm hồi quy.
        target.write_text((ROOT / "src/chat/contacts.rs").read_text())
        print("RUN đối chứng dương: sáu ca danh bạ với mã gốc", flush=True)
        positive = subprocess.run(["./build.sh", "test"], cwd=snapshot,
                                  env={**os.environ, "TGGW_BUILD_DIR": "/opt/tggw-build-danhba"},
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        print(positive.stdout, end="", flush=True)
        assert positive.returncode == 0, f"Đối chứng dương trả mã {positive.returncode}"
        assert "6 passed; 0 failed;" in positive.stdout, "Thiếu sáu ca danh bạ đạt"
    assert fingerprint() == source, "Mã gốc thay đổi sau khi kiểm đột biến"
    print("PASS: TC-1 và TC-5 đều bắt đột biến; sáu ca mã gốc đạt; mã gốc giữ nguyên.", flush=True)


if __name__ == "__main__":
    main()
