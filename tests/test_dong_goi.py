"""Đóng gói Docker/add-on: trần tài nguyên (docs/QUY-TAC-TAI-NGUYEN.md)."""
import re
import unittest
from pathlib import Path

GOC = Path(__file__).resolve().parents[1]


def doc(ten: str) -> str:
    return (GOC / ten).read_text(encoding="utf-8")


def gia_tri(van_ban: str, khoa: str) -> str:
    m = re.search(rf"^\s*{re.escape(khoa)}:\s*[\"']?([^\"'\n#]+?)[\"']?\s*(?:#.*)?$", van_ban, re.M)
    assert m, f"thiếu {khoa}"
    return m.group(1).strip()


class DongGoiTeleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compose = doc("docker-compose.example.yml")
        cls.config = doc("addon/tele-rs/config.yaml")

    def test_compose_co_tran_tai_nguyen(self):
        self.assertRegex(self.compose, r"(?m)^\s*init:\s*true")
        self.assertEqual(gia_tri(self.compose, "mem_limit"), "1g")
        self.assertEqual(gia_tri(self.compose, "memswap_limit"), "1g")
        self.assertEqual(gia_tri(self.compose, "mem_limit"), gia_tri(self.compose, "memswap_limit"))
        self.assertEqual(gia_tri(self.compose, "pids_limit"), "256")
        self.assertIn("no-new-privileges:true", self.compose)

    def test_compose_xoay_vong_nhat_ky(self):
        self.assertEqual(gia_tri(self.compose, "driver"), "json-file")
        self.assertEqual(gia_tri(self.compose, "max-size"), "10m")
        self.assertEqual(gia_tri(self.compose, "max-file"), "3")

    def test_addon_config_bat_init(self):
        self.assertEqual(gia_tri(self.config, "init"), "true")


if __name__ == "__main__":
    unittest.main()
