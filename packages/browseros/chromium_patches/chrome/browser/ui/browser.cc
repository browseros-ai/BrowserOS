diff --git a/chrome/browser/ui/browser.cc b/chrome/browser/ui/browser.cc
index ca32d6faace3a..459c9597ea6f8 100644
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
@@ -2298,6 +2299,11 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
       source->GetController().GetPendingEntry()
           ? source->GetController().GetPendingEntry()
           : source->GetController().GetLastCommittedEntry();
+
+  // BrowserOS: Check once so the per-URL gates below can use it.
+  const bool ntp_focus_content =
+      browseros::IsNtpFocusContentEnabled(profile_->GetPrefs());
+
   if (entry) {
     const GURL& url = entry->GetURL();
     const GURL& virtual_url = entry->GetVirtualURL();
@@ -2310,15 +2316,18 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
          url.host() == chrome::kChromeUINewTabHost) ||
         (virtual_url.SchemeIs(content::kChromeUIScheme) &&
          virtual_url.host() == chrome::kChromeUINewTabHost)) {
-      return true;
+      return !ntp_focus_content;
     }
 
     if (url.spec() == chrome::kChromeUISplitViewNewTabPageURL) {
-      return true;
+      return !ntp_focus_content;
     }
   }
 
-  return search::NavEntryIsInstantNTP(source, entry);
+  if (search::NavEntryIsInstantNTP(source, entry)) {
+    return !ntp_focus_content;
+  }
+  return false;
 }
 
 bool Browser::ShouldFocusPageAfterCrash(WebContents* source) {
