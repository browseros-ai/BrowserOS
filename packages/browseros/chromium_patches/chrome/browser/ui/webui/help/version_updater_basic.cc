diff --git a/chrome/browser/ui/webui/help/version_updater_basic.cc b/chrome/browser/ui/webui/help/version_updater_basic.cc
index ebcef5637b150..27d1abebda5c7 100644
--- a/chrome/browser/ui/webui/help/version_updater_basic.cc
+++ b/chrome/browser/ui/webui/help/version_updater_basic.cc
@@ -5,9 +5,16 @@
 #include <memory>
 #include <string>
 
+#include "base/logging.h"
+#include "chrome/browser/buildflags.h"
 #include "chrome/browser/ui/webui/help/version_updater.h"
 #include "chrome/browser/upgrade_detector/upgrade_detector.h"
 
+#if BUILDFLAG(ENABLE_WINSPARKLE)
+#include "chrome/browser/ui/webui/help/winsparkle_version_updater_win.h"
+#include "chrome/browser/win/winsparkle_glue.h"
+#endif
+
 namespace {
 
 // Bare bones implementation just checks if a new version is ready.
@@ -31,5 +38,14 @@ class VersionUpdaterBasic : public VersionUpdater {
 
 std::unique_ptr<VersionUpdater> VersionUpdater::Create(
     content::WebContents* web_contents) {
+#if BUILDFLAG(ENABLE_WINSPARKLE)
+  // Mirror the macOS Sparkle behavior: prefer WinSparkle when it came up,
+  // fall back to the basic updater otherwise.
+  if (winsparkle_glue::IsEnabled()) {
+    VLOG(1) << "VersionUpdater: Using WinSparkle updater";
+    return std::make_unique<WinSparkleVersionUpdater>();
+  }
+  VLOG(1) << "VersionUpdater: WinSparkle not available, using basic updater";
+#endif
   return std::make_unique<VersionUpdaterBasic>();
 }
