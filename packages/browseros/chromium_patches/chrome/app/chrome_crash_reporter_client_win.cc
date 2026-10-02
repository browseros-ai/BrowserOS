diff --git a/chrome/app/chrome_crash_reporter_client_win.cc b/chrome/app/chrome_crash_reporter_client_win.cc
index f7ce95d3977ce5d0dc3193a11083209f151f2a14..eeb01977a7d97de52fd7d59570b52a9954b4114f 100644
--- a/chrome/app/chrome_crash_reporter_client_win.cc
+++ b/chrome/app/chrome_crash_reporter_client_win.cc
@@ -29,6 +29,12 @@
 #include "components/metrics/system_profile_user_stream.h"
 #include "components/version_info/channel.h"
 
+namespace {
+constexpr char kSentryMinidumpUrl[] =
+    "https://o4510545525932032.ingest.us.sentry.io/api/4510938172620800/"
+    "minidump/?sentry_key=9a76046fcfbcfe69a3580f4d204579f1";
+}  // namespace
+
 ChromeCrashReporterClient::ChromeCrashReporterClient() = default;
 
 ChromeCrashReporterClient::~ChromeCrashReporterClient() = default;
@@ -95,9 +101,8 @@ void ChromeCrashReporterClient::GetProductInfo(ProductInfo* product_info) {
   GetProductNameAndVersion(exe_file, &product_name, &version, &special_build,
                            &channel_name);
 
-  *product_info =
-      ProductInfo(base::WideToUTF8(product_name), base::WideToUTF8(version),
-                  base::WideToUTF8(channel_name));
+  *product_info = ProductInfo("BrowserOS", base::WideToUTF8(version),
+                             base::WideToUTF8(channel_name));
 }
 
 bool ChromeCrashReporterClient::GetShouldDumpLargerDumps() {
@@ -146,7 +151,8 @@ bool ChromeCrashReporterClient::IsRunningUnattended() {
 }
 
 bool ChromeCrashReporterClient::GetCollectStatsConsent() {
-  return install_static::GetCollectStatsConsent();
+  // Enable crash reporting.
+  return true;
 }
 
 bool ChromeCrashReporterClient::GetCollectStatsInSample() {
@@ -236,3 +242,7 @@ std::wstring ChromeCrashReporterClient::GetWerRuntimeExceptionModule() {
   // file_start points to the start of the filename in the elf_dir buffer.
   return std::wstring(elf_dir, file_start).append(kWerDll);
 }
+
+std::string ChromeCrashReporterClient::GetUploadUrl() {
+  return kSentryMinidumpUrl;
+}
