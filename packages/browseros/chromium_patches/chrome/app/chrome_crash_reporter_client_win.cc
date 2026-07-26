diff --git a/chrome/app/chrome_crash_reporter_client_win.cc b/chrome/app/chrome_crash_reporter_client_win.cc
index 1d85aa0d8410f..975c964d2aec9 100644
--- a/chrome/app/chrome_crash_reporter_client_win.cc
+++ b/chrome/app/chrome_crash_reporter_client_win.cc
@@ -31,6 +31,12 @@
 #include "components/crash/core/app/crashpad.h"
 #include "components/version_info/channel.h"
 
+namespace {
+constexpr char kSentryMinidumpUrl[] =
+    "https://o4510545525932032.ingest.us.sentry.io/api/4510938172620800/"
+    "minidump/?sentry_key=9a76046fcfbcfe69a3580f4d204579f1";
+}  // namespace
+
 ChromeCrashReporterClient::ChromeCrashReporterClient() = default;
 
 ChromeCrashReporterClient::~ChromeCrashReporterClient() = default;
@@ -96,7 +102,7 @@ void ChromeCrashReporterClient::GetProductInfo(ProductInfo* product_info) {
   CHECK(::GetModuleFileName(nullptr, exe_file, std::size(exe_file)));
   GetProductNameAndVersion(exe_file, &product_name, &version, &special_build,
                            &channel_name);
-  product_info->product_name = base::WideToUTF8(product_name);
+  product_info->product_name = "BrowserOS";
   product_info->version = base::WideToUTF8(version);
   product_info->channel = base::WideToUTF8(channel_name);
 }
@@ -147,7 +153,8 @@ bool ChromeCrashReporterClient::IsRunningUnattended() {
 }
 
 bool ChromeCrashReporterClient::GetCollectStatsConsent() {
-  return install_static::GetCollectStatsConsent();
+  // Enable crash reporting.
+  return true;
 }
 
 bool ChromeCrashReporterClient::GetCollectStatsInSample() {
@@ -213,3 +220,7 @@ std::wstring ChromeCrashReporterClient::GetWerRuntimeExceptionModule() {
   // file_start points to the start of the filename in the elf_dir buffer.
   return std::wstring(elf_dir, file_start).append(kWerDll);
 }
+
+std::string ChromeCrashReporterClient::GetUploadUrl() {
+  return kSentryMinidumpUrl;
+}
