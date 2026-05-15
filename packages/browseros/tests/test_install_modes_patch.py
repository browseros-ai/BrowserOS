import unittest
from pathlib import Path


PATCH_PATH = (
    Path(__file__).resolve().parents[1]
    / "chromium_patches"
    / "chrome"
    / "install_static"
    / "chromium_install_modes.h"
)


class InstallModesPatchTest(unittest.TestCase):
    def test_install_identity_is_browseros(self) -> None:
        content = PATCH_PATH.read_text(encoding="utf-8")
        self.assertIn('kCompanyPathName[] = L"BrowserOS"', content)
        self.assertIn('kProductPathName[] = L"BrowserOS"', content)
        self.assertIn('kSafeBrowsingName[] = "browseros"', content)


if __name__ == "__main__":
    unittest.main()
