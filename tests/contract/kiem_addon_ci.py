#!/usr/bin/env python3
"""Kiểm cấu trúc add-on Home Assistant, Dockerfile, tài liệu và GitHub Actions cho tele-rust."""

import ipaddress
from pathlib import Path
import re
import struct
import sys
import yaml


ROOT = Path(__file__).resolve().parents[2]
EXPECTED_WORKFLOWS = {"addon.yml", "ci.yml", "image.yml", "release.yml", "auto-release.yml"}
SECRET_ADDON_OPTIONS = {"tg_api_hash", "gateway_token", "tg_ui_password", "ha_webhook_token"}
ALLOWED_NON_DOC_IPS = {"0.0.0.0", "127.0.0.1"}
DOC_IP_NETS = [
    ipaddress.ip_network("192.0.2.0/24"),
    ipaddress.ip_network("198.51.100.0/24"),
    ipaddress.ip_network("203.0.113.0/24"),
]


class UniqueKeyLoader(yaml.SafeLoader):
    """YAML loader từ chối khoá trùng lặp trong cùng mapping."""


def _construct_mapping(loader, node, deep=False):
    seen = set()
    for key_node, _ in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in seen:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                f"found duplicate key {key!r}",
                key_node.start_mark,
            )
        seen.add(key)
    return super(UniqueKeyLoader, loader).construct_mapping(node, deep=deep)


UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    _construct_mapping,
)


def parse_cargo_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"', text, re.M)
    assert match, "Không tìm thấy version trong Cargo.toml"
    return match.group(1)


def read_yaml_checked(path: Path):
    text = path.read_text(encoding="utf-8")
    assert text.endswith("\n"), f"{path.relative_to(ROOT)} thiếu ký tự xuống dòng cuối tệp"
    assert not text.endswith("\n\n"), f"{path.relative_to(ROOT)} dư dòng trống cuối tệp"
    for lineno, line in enumerate(text.splitlines(), 1):
        assert "\t" not in line, f"{path.relative_to(ROOT)}:{lineno} chứa ký tự tab"
        assert line == line.rstrip(" \t"), f"{path.relative_to(ROOT)}:{lineno} có khoảng trắng cuối dòng"
        assert len(line) <= 160, f"{path.relative_to(ROOT)}:{lineno} dài {len(line)} > 160 ký tự"
    return yaml.load(text, Loader=UniqueKeyLoader)


def read_png_size(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", f"{path.relative_to(ROOT)} không phải PNG hợp lệ"
    assert data[12:16] == b"IHDR", f"{path.relative_to(ROOT)} thiếu header IHDR"
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def collect_src_env_vars() -> set[str]:
    vars_found = set()
    for p in (ROOT / "src").rglob("*.rs"):
        if p.name == "tests.rs" or p.name == "fake.rs" or "tests" in p.relative_to(ROOT / "src").parts:
            continue
        text = p.read_text(encoding="utf-8")
        vars_found.update(re.findall(r'(?:std::env::var(?:_os)?|env_num|number|get)\(\s*"([A-Z][A-Z0-9_]{2,})"', text))
        for a, b in re.findall(r'alias\(\s*"([A-Z][A-Z0-9_]{2,})"\s*,\s*"([A-Z][A-Z0-9_]{2,})"', text):
            vars_found.add(a)
            vars_found.add(b)
    return vars_found


def check_repository(repo_path: Path) -> None:
    data = read_yaml_checked(repo_path)
    assert isinstance(data, dict), "repository.yaml phải là mapping"
    for key in ("name", "url", "maintainer"):
        val = data.get(key)
        assert isinstance(val, str) and val.strip(), f"repository.yaml thiếu trường {key}"
    assert "TriTue2011/tele-rust" in data["url"], f"repository.yaml url không khớp: {data['url']}"


def check_addon(cargo_ver: str, src_vars: set[str]) -> dict:
    addon_dir = ROOT / "addon" / "tele-rs"
    cfg = read_yaml_checked(addon_dir / "config.yaml")
    assert isinstance(cfg, dict), "addon/tele-rs/config.yaml phải là mapping"
    required = ("name", "description", "version", "slug", "arch", "image", "ports", "map", "options", "schema")
    for key in required:
        assert key in cfg, f"addon/tele-rs/config.yaml thiếu {key}"

    assert str(cfg["version"]) == cargo_ver, f"addon version {cfg['version']} != Cargo.toml {cargo_ver}"
    assert re.fullmatch(r"[a-z0-9_]+", cfg["slug"]), f"slug không hợp lệ: {cfg['slug']}"
    assert cfg["arch"] == ["amd64", "aarch64"], f"arch phải đúng amd64+aarch64 (NTgCalls/TDLib đều có bản arm64): {cfg['arch']}"
    assert cfg["image"] == "ghcr.io/tritue2011/tele-rust", f"image không khớp: {cfg['image']}"
    assert not cfg.get("ingress"), "Không bật ingress khi gateway dùng CSP frame-ancestors 'none' và /api/* gốc"
    assert "3334/tcp" in cfg["ports"], "ports thiếu 3334/tcp"
    map_entries = {str(item).split(":", 1)[0] for item in cfg["map"]}
    assert "data" in map_entries, f"map thiếu data: {cfg['map']}"

    options = cfg["options"]
    schema = cfg["schema"]
    assert isinstance(options, dict) and isinstance(schema, dict)
    assert set(options.keys()) == set(schema.keys()), (
        f"options và schema lệch khoá: {set(options) ^ set(schema)}"
    )
    for secret_key in SECRET_ADDON_OPTIONS:
        assert secret_key in schema, f"thiếu tuỳ chọn bí mật {secret_key}"
        assert schema[secret_key] == "password?", f"schema.{secret_key} phải là password?, nhận {schema[secret_key]}"

    entrypoint_text = (ROOT / "docker" / "entrypoint.sh").read_text(encoding="utf-8")
    mapped = {}
    for opt, env_var in re.findall(
        r'set_(?:str|bool|int|int_pos)_opt\s+"([a-z0-9_]+)"\s+"([A-Z][A-Z0-9_]+)"',
        entrypoint_text,
    ):
        mapped[opt] = env_var
    if 'opt_raw "video_profile"' in entrypoint_text and "VIDEO_PROFILE" in entrypoint_text:
        mapped["video_profile"] = "VIDEO_PROFILE"
    if 'opt_raw "webrtc_log"' in entrypoint_text and "TELEGRAM_WEBRTC_LOG" in entrypoint_text:
        mapped["webrtc_log"] = "TELEGRAM_WEBRTC_LOG"

    assert set(mapped.keys()) == set(options.keys()), (
        f"entrypoint.sh ánh xạ {set(mapped)} khác options {set(options)}"
    )
    missing_in_src = set(mapped.values()) - src_vars
    assert not missing_in_src, f"entrypoint.sh xuất biến không có trong src/: {sorted(missing_in_src)}"

    readme_addon = (addon_dir / "README.md").read_text(encoding="utf-8")
    docs_addon = (addon_dir / "DOCS.md").read_text(encoding="utf-8")
    assert len(readme_addon.strip()) > 100, "addon/tele-rs/README.md quá ngắn hoặc rỗng"
    assert len(docs_addon.strip()) > 300, "addon/tele-rs/DOCS.md quá ngắn hoặc rỗng"
    for opt, env_var in mapped.items():
        assert f"`{opt}`" in docs_addon, f"DOCS.md thiếu mô tả tuỳ chọn {opt}"
        assert f"`{env_var}`" in docs_addon, f"DOCS.md thiếu biến môi trường {env_var}"

    assert read_png_size(addon_dir / "icon.png") == (128, 128), "icon.png phải có kích thước 128x128"
    assert read_png_size(addon_dir / "logo.png") == (256, 128), "logo.png phải có kích thước 256x128"
    return cfg


def check_root_readme_and_docs(cargo_ver: str, src_vars: set[str]) -> None:
    readme_text = (ROOT / "README.md").read_text(encoding="utf-8")
    assert f"v{cargo_ver}" in readme_text, f"README.md thiếu phiên bản v{cargo_ver}"
    assert "ghcr.io/tritue2011/tele-rust" in readme_text, "README.md thiếu tên ảnh ghcr.io/tritue2011/tele-rust"
    assert "macOS" in readme_text, "README.md thiếu mục trạng thái macOS"
    assert "MIT" in readme_text and "LGPL-3.0" in readme_text and "BSL-1.0" in readme_text

    ref_text = (ROOT / "docs" / "THAM-CHIEU.md").read_text(encoding="utf-8")
    m = re.search(r"\n## Biến môi trường\n([\s\S]*?)(?=\n## |\Z)", ref_text)
    assert m, "docs/THAM-CHIEU.md thiếu mục ## Biến môi trường"
    readme_vars = set()
    for line in m.group(1).splitlines():
        s = line.strip()
        if not s.startswith("|") or s.startswith("| ---") or s.startswith("| Biến"):
            continue
        cols = [c.strip() for c in s.split("|")[1:-1]]
        for col in cols[:2]:
            readme_vars.update(re.findall(r"`([A-Z][A-Z0-9_]{2,})`", col))
    assert readme_vars, "Không đọc được biến nào từ bảng ## Biến môi trường trong docs/THAM-CHIEU.md"
    unknown = readme_vars - src_vars
    assert not unknown, f"docs/THAM-CHIEU.md nhắc biến không có trong src/: {sorted(unknown)}"

    for rel in ("LICENSE", "NOTICE.md", "docs/API.md", "docs/TRIEN-KHAI.md", "docs/BAO-MAT.md", "docs/CHANGELOG.md", "docs/THAM-CHIEU.md", "docs/TINH-NANG.md"):
        p = ROOT / rel
        assert p.is_file() and len(p.read_text(encoding="utf-8").strip()) > 100, f"Thiếu hoặc rỗng {rel}"

    notice = (ROOT / "NOTICE.md").read_text(encoding="utf-8")
    for kw in ("NTgCalls", "LGPL-3.0", "TDLib", "BSL-1.0", "liên kết lại"):
        assert kw in notice, f"NOTICE.md thiếu nội dung bắt buộc: {kw}"

    assert not (ROOT / "HOAN-TAT.md.SAI-212357").exists(), "Tệp nội bộ HOAN-TAT.md.SAI-212357 chưa bị xoá"


def check_dockerfile() -> None:
    text = (ROOT / "Dockerfile").read_text(encoding="utf-8")
    stages = re.findall(r"^FROM\s+\S+(?:\s+AS\s+(\S+))?", text, re.M)
    assert len(stages) >= 3, f"Dockerfile phải là multi-stage (>=3 tầng), thấy {len(stages)} stage"
    user_matches = re.findall(r"^USER\s+(\S+)", text, re.M)
    assert user_matches, "Dockerfile thiếu lệnh USER không phải root"
    assert user_matches[-1].split(":")[0] not in ("root", "0"), f"USER cuối cùng vẫn là root: {user_matches[-1]}"
    assert re.search(r"^VOLUME\b", text, re.M), "Dockerfile thiếu VOLUME"
    assert re.search(r"^EXPOSE\s+3334\b", text, re.M), "Dockerfile thiếu EXPOSE 3334"
    assert re.search(r"^HEALTHCHECK\b", text, re.M), "Dockerfile thiếu HEALTHCHECK"
    assert re.search(r"^ENTRYPOINT\b", text, re.M), "Dockerfile thiếu ENTRYPOINT"
    for kw in ("ffmpeg", "TDJSON_VERSION", "WITH_TTS", "edge-tts", "/licenses", "LICENSE", "NOTICE.md"):
        assert kw in text, f"Dockerfile thiếu thành phần bắt buộc: {kw}"
    assert (ROOT / ".dockerignore").is_file(), "Thiếu tệp .dockerignore"


def _walk_steps(node):
    if isinstance(node, dict):
        if "steps" in node and isinstance(node["steps"], list):
            for step in node["steps"]:
                yield step
        for val in node.values():
            yield from _walk_steps(val)
    elif isinstance(node, list):
        for item in node:
            yield from _walk_steps(item)


def check_workflows() -> list[Path]:
    wf_dir = ROOT / ".github" / "workflows"
    files = sorted(wf_dir.glob("*.yml"))
    names = {p.name for p in files}
    assert names == EXPECTED_WORKFLOWS, f"Danh sách workflow {names} khác kỳ vọng {EXPECTED_WORKFLOWS}"

    pin_re = re.compile(r"^[^@]+@(?:v[0-9]+|[0-9a-f]{40})$")
    inject_re = re.compile(r"\$\{\{\s*(?:github\.event\.inputs\.|inputs\.)")
    bad_secret_re = re.compile(r"secrets\.(?!GITHUB_TOKEN\b)[A-Z0-9_]+")

    for wf in files:
        raw_text = wf.read_text(encoding="utf-8")
        assert not bad_secret_re.search(raw_text), f"{wf.name} chỉ được dùng secrets.GITHUB_TOKEN"
        data = read_yaml_checked(wf)
        assert isinstance(data, dict), f"{wf.name} phải là mapping YAML"
        assert "permissions" in data, f"{wf.name} thiếu permissions tối thiểu ở mức workflow"
        assert data["permissions"] != "write-all", f"{wf.name} không được dùng write-all"

        for job_name, job in (data.get("jobs") or {}).items():
            if isinstance(job, dict) and "permissions" in job:
                assert job["permissions"] != "write-all", f"{wf.name}:{job_name} dùng write-all"

        for step in _walk_steps(data):
            if not isinstance(step, dict):
                continue
            uses = step.get("uses")
            if isinstance(uses, str) and not uses.startswith("./"):
                assert pin_re.match(uses), f"{wf.name} có action chưa ghim thẻ chính/SHA: {uses}"
            run_cmd = step.get("run")
            if isinstance(run_cmd, str):
                assert not inject_re.search(run_cmd), (
                    f"{wf.name} nội suy trực tiếp inputs trong run: (nguy cơ script injection)"
                )

        if wf.name == "release.yml":
            assert ".dmg" not in raw_text and "cargo build" not in raw_text, (
                "release.yml không được dựng hoặc đính kèm nhị phân trần"
            )
    return files


def check_no_internal_traces_and_ips(targets: list[Path]) -> None:
    forbidden = [
        "".join(["172.", "16."]),
        "".join(["ha", "200"]),
        "".join(["/opt/", "claude-c2a"]),
        "".join(["/opt/", "telegram-gateway-rs-build"]),
        "".join(["tggw-", "build-h2"]),
        "".join(["tham-khao-", "wf"]),
        "".join(["@gm", "ail"]),
    ]
    pattern = re.compile("|".join(re.escape(item) for item in forbidden), re.IGNORECASE)
    ip_re = re.compile(r"\b(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})(/16)?\b")
    for path in targets:
        if path.suffix == ".png":
            continue
        text = path.read_text(encoding="utf-8")
        match = pattern.search(text)
        assert not match, f"{path.relative_to(ROOT)} còn dấu vết nội bộ: {match.group(0)}"
        for ip_str, cidr16 in ip_re.findall(text):
            if ip_str in ALLOWED_NON_DOC_IPS or (ip_str == "".join(["169.254", ".0.0"]) and cidr16 == "/16"):
                continue
            addr = ipaddress.ip_address(ip_str)
            assert any(addr in net for net in DOC_IP_NETS), (
                f"{path.relative_to(ROOT)} chứa IPv4 ngoài dải tài liệu RFC 5737: {ip_str}"
            )


def main() -> int:
    cargo_ver = parse_cargo_version()
    src_vars = collect_src_env_vars()

    check_repository(ROOT / "repository.yaml")
    check_addon(cargo_ver, src_vars)
    check_root_readme_and_docs(cargo_ver, src_vars)
    check_dockerfile()
    wf_files = check_workflows()

    scan_targets = [
        ROOT / "Dockerfile",
        ROOT / ".dockerignore",
        ROOT / "docker" / "entrypoint.sh",
        ROOT / "repository.yaml",
        ROOT / "LICENSE",
        ROOT / "NOTICE.md",
        ROOT / "README.md",
        *sorted((ROOT / "docs").glob("*.md")),
        *sorted((ROOT / "addon" / "tele-rs").rglob("*")),
        *wf_files,
        ROOT / "tests" / "readme-matches-code.test.cjs",
        Path(__file__).resolve(),
    ]
    check_no_internal_traces_and_ips([p for p in scan_targets if p.is_file()])
    print(f"ADDON_CI_CHECK_OK=1; VERSION={cargo_ver}; WORKFLOWS={len(wf_files)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
