diff --git a/chrome/install_static/chromium_install_modes.cc b/chrome/install_static/chromium_install_modes.cc
index f5af44ca1c489..62378ff18683c 100644
--- a/chrome/install_static/chromium_install_modes.cc
+++ b/chrome/install_static/chromium_install_modes.cc
@@ -12,12 +12,12 @@
 
 namespace install_static {
 
-const wchar_t kCompanyPathName[] = L"";
+const wchar_t kCompanyPathName[] = L"BrowserOS";
 
-const wchar_t kProductPathName[] = L"Chromium";
+const wchar_t kProductPathName[] = L"BrowserOS";
 
 const size_t kProductPathNameLength = _countof(kProductPathName) - 1;
 
-const char kSafeBrowsingName[] = "chromium";
+const char kSafeBrowsingName[] = "browseros";
 
 }  // namespace install_static
