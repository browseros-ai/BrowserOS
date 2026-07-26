diff --git a/chrome/common/chrome_paths_linux.cc b/chrome/common/chrome_paths_linux.cc
index 5575c049b222f..800d48d3c798d 100644
--- a/chrome/common/chrome_paths_linux.cc
+++ b/chrome/common/chrome_paths_linux.cc
@@ -15,6 +15,7 @@
 #include "base/strings/string_util.h"
 #include "build/branding_buildflags.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/buildflags.h"
 #include "chrome/common/channel_info.h"
 #include "chrome/common/chrome_paths_internal.h"
 
@@ -94,8 +95,10 @@ bool GetDefaultUserDataDirectory(base::FilePath* result) {
   std::string data_dir_basename = "google-chrome-for-testing";
 #elif BUILDFLAG(GOOGLE_CHROME_BRANDING)
   std::string data_dir_basename = "google-chrome";
+#elif BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+  std::string data_dir_basename = "browser-claw";
 #else
-  std::string data_dir_basename = "chromium";
+  std::string data_dir_basename = "browser-os";
 #endif
   *result = config_dir.Append(data_dir_basename + GetChannelSuffixForDataDir());
   return true;
