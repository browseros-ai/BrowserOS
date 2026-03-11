diff --git a/chrome/browser/ui/browser.cc b/chrome/browser/ui/browser.cc
index ca32d6faace3a..834d85247869c 100644
--- a/chrome/browser/ui/browser.cc
+++ b/chrome/browser/ui/browser.cc
@@ -42,6 +42,7 @@
 #include "chrome/browser/background/background_contents_service_factory.h"
 #include "chrome/browser/bookmarks/bookmark_model_factory.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/buildflags.h"
 #include "chrome/browser/content_settings/host_content_settings_map_factory.h"
 #include "chrome/browser/content_settings/mixed_content_settings_tab_helper.h"
@@ -2298,6 +2299,12 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
       source->GetController().GetPendingEntry()
           ? source->GetController().GetPendingEntry()
           : source->GetController().GetLastCommittedEntry();
+  // BrowserOS: When enabled, skip all NTP-specific focus-to-omnibox logic
+  // below, so the NTP web contents receives focus instead.
+  if (browseros::IsNtpFocusContentEnabled(profile_->GetPrefs())) {
+    return false;
+  }
+
   if (entry) {
     const GURL& url = entry->GetURL();
     const GURL& virtual_url = entry->GetVirtualURL();
