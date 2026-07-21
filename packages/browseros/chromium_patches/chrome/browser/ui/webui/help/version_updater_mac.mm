diff --git a/chrome/browser/ui/webui/help/version_updater_mac.mm b/chrome/browser/ui/webui/help/version_updater_mac.mm
index 57648956faf5058fada6b02abd3301deebb5a295..c0e4f6aa2bf4f14ef2df49c692e3601b77054efb 100644
--- a/chrome/browser/ui/webui/help/version_updater_mac.mm
+++ b/chrome/browser/ui/webui/help/version_updater_mac.mm
@@ -6,6 +6,15 @@
 
 #import <Foundation/Foundation.h>
 
+// Include Sparkle updater if available
+#include "base/command_line.h"
+#include "chrome/browser/buildflags.h"
+
+#if BUILDFLAG(ENABLE_SPARKLE)
+#include "chrome/browser/ui/webui/help/sparkle_version_updater_mac.h"
+#include "chrome/browser/mac/sparkle_glue.h"
+#endif
+
 #include <algorithm>
 #include <memory>
 #include <string>
@@ -76,6 +85,8 @@ void UpdateStatus(VersionUpdater::StatusCallback status_callback,
                    : VersionUpdater::Status::UPDATED;
       break;
     case updater::UpdateService::UpdateState::State::kUpdateError:
+      // Log only errors
+      VLOG(1) << "Update error, code: " << update_state.error_code;
       switch (update_state.error_code) {
         case updater::GOOPDATE_E_APP_UPDATE_DISABLED_BY_POLICY:
           status = VersionUpdater::Status::DISABLED_BY_ADMIN;
@@ -133,5 +144,18 @@ class VersionUpdaterMac : public VersionUpdater {
 
 std::unique_ptr<VersionUpdater> VersionUpdater::Create(
     content::WebContents* /* web_contents */) {
+#if BUILDFLAG(ENABLE_SPARKLE)
+  // Use Sparkle updater if it's enabled
+  if (sparkle_glue::SparkleEnabled()) {
+    LOG(INFO) << "VersionUpdater: Using Sparkle updater";
+    return base::WrapUnique(new SparkleVersionUpdater());
+  }
+  else {
+    LOG(INFO) << "VersionUpdater: Sparkle updater not available, using default updater";
+  }
+#endif
+
+  LOG(INFO) << "VersionUpdater: Using default Chromium updater";
+  // Otherwise use the default Chromium updater
   return base::WrapUnique(new VersionUpdaterMac());
 }
