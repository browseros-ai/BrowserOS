diff --git a/chrome/app/chrome_crash_reporter_client.cc b/chrome/app/chrome_crash_reporter_client.cc
index f0c0ff0871ee9..61e234d740980 100644
--- a/chrome/app/chrome_crash_reporter_client.cc
+++ b/chrome/app/chrome_crash_reporter_client.cc
@@ -16,6 +16,7 @@
 #include "base/strings/utf_string_conversions.h"
 #include "build/branding_buildflags.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/common/channel_info.h"
 #include "chrome/common/chrome_paths.h"
 #include "chrome/common/chrome_paths_internal.h"
@@ -131,19 +132,19 @@ void ChromeCrashReporterClient::GetProductInfo(ProductInfo* product_info) {
   CHECK(product_info);
 
 #if BUILDFLAG(IS_ANDROID)
-  product_info->product_name = "Chrome_Android";
+  product_info->product_name = "BrowserOS_Android";
 #elif BUILDFLAG(IS_CHROMEOS)
-  product_info->product_name = "Chrome_ChromeOS";
+  product_info->product_name = "BrowserOS_ChromeOS";
 #elif BUILDFLAG(IS_LINUX)
 #if defined(ADDRESS_SANITIZER)
-  product_info->product_name = "Chrome_Linux_ASan";
+  product_info->product_name = "BrowserOS_Linux_ASan";
 #else
-  product_info->product_name = "Chrome_Linux";
+  product_info->product_name = "BrowserOS_Linux";
 #endif  // defined(ADDRESS_SANITIZER)
 #elif BUILDFLAG(IS_MAC)
-  product_info->product_name = "Chrome_Mac";
+  product_info->product_name = "BrowserOS_Mac";
 #elif BUILDFLAG(IS_WIN)
-  product_info->product_name = "Chrome";
+  product_info->product_name = "BrowserOS";
 #else
   NOTREACHED();
 #endif
@@ -169,42 +170,8 @@ bool ChromeCrashReporterClient::IsRunningUnattended() {
 }
 
 bool ChromeCrashReporterClient::GetCollectStatsConsent() {
-#if BUILDFLAG(GOOGLE_CHROME_BRANDING)
-  bool is_official_chrome_build = true;
-#else
-  bool is_official_chrome_build = false;
-#endif
-
-#if BUILDFLAG(IS_CHROMEOS)
-  bool is_guest_session = base::CommandLine::ForCurrentProcess()->HasSwitch(
-      ash::switches::kGuestSession);
-  bool is_stable_channel =
-      chrome::GetChannel() == version_info::Channel::STABLE;
-
-  if (is_guest_session && is_stable_channel) {
-    VLOG(1) << "GetCollectStatsConsent(): is_guest_session " << is_guest_session
-            << " && is_stable_channel " << is_stable_channel
-            << " so returning false";
-    return false;
-  }
-#endif  // BUILDFLAG(IS_CHROMEOS)
-
-#if BUILDFLAG(IS_ANDROID)
-  // TODO(jcivelli): we should not initialize the crash-reporter when it was not
-  // enabled. Right now if it is disabled we still generate the minidumps but we
-  // do not upload them.
-  return is_official_chrome_build;
-#else   // !BUILDFLAG(IS_ANDROID)
-  if (!is_official_chrome_build) {
-    VLOG(1) << "GetCollectStatsConsent(): is_official_chrome_build is false "
-            << "so returning false";
-    return false;
-  }
-  bool settings_consent = GoogleUpdateSettings::GetCollectStatsConsent();
-  VLOG(1) << "GetCollectStatsConsent(): settings_consent: " << settings_consent
-          << " so returning that";
-  return settings_consent;
-#endif  // BUILDFLAG(IS_ANDROID)
+  // Enable crash reporting.
+  return true;
 }
 
 #if BUILDFLAG(IS_LINUX) || BUILDFLAG(IS_CHROMEOS)
@@ -223,3 +190,7 @@ bool ChromeCrashReporterClient::EnableBreakpadForProcess(
          process_type == switches::kGpuProcess ||
          process_type == switches::kUtilityProcess;
 }
+
+std::string ChromeCrashReporterClient::GetUploadUrl() {
+  return browseros::kSentryMinidumpUrl;
+}
