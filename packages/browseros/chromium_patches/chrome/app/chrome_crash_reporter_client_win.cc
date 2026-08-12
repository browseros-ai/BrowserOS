diff --git a/chrome/app/chrome_crash_reporter_client_win.cc b/chrome/app/chrome_crash_reporter_client_win.cc
index e59af188c8140cbbc16f14bf49ef162cace60dc9..35320334dd983e87415fe1166edd68c2829bab60 100644
--- a/chrome/app/chrome_crash_reporter_client_win.cc
+++ b/chrome/app/chrome_crash_reporter_client_win.cc
@@ -26,6 +26,12 @@
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
@@ -91,7 +97,7 @@ void ChromeCrashReporterClient::GetProductInfo(ProductInfo* product_info) {
   CHECK(::GetModuleFileName(nullptr, exe_file, std::size(exe_file)));
   GetProductNameAndVersion(exe_file, &product_name, &version, &special_build,
                            &channel_name);
-  product_info->product_name = base::WideToUTF8(product_name);
+  product_info->product_name = "BrowserOS";
   product_info->version = base::WideToUTF8(version);
   product_info->channel = base::WideToUTF8(channel_name);
 }
@@ -142,7 +148,8 @@ bool ChromeCrashReporterClient::IsRunningUnattended() {
 }
 
 bool ChromeCrashReporterClient::GetCollectStatsConsent() {
-  return install_static::GetCollectStatsConsent();
+  // Enable crash reporting.
+  return true;
 }
 
 bool ChromeCrashReporterClient::GetCollectStatsInSample() {
@@ -208,3 +215,7 @@ std::wstring ChromeCrashReporterClient::GetWerRuntimeExceptionModule() {
   // file_start points to the start of the filename in the elf_dir buffer.
   return std::wstring(elf_dir, file_start).append(kWerDll);
 }
+
+std::string ChromeCrashReporterClient::GetUploadUrl() {
+  return kSentryMinidumpUrl;
+}
