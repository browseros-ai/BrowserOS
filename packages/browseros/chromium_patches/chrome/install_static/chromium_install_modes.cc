diff --git a/chrome/install_static/chromium_install_modes.cc b/chrome/install_static/chromium_install_modes.cc
index f5af44ca1c489..57dc381b0147f 100644
--- a/chrome/install_static/chromium_install_modes.cc
+++ b/chrome/install_static/chromium_install_modes.cc
@@ -12,9 +12,9 @@
 
 namespace install_static {
 
-const wchar_t kCompanyPathName[] = L"";
+const wchar_t kCompanyPathName[] = L"BlockBrowser";
 
-const wchar_t kProductPathName[] = L"Chromium";
+const wchar_t kProductPathName[] = L"BlockBrowser";
 
 const size_t kProductPathNameLength = _countof(kProductPathName) - 1;
 
