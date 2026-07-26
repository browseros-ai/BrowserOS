diff --git a/chrome/browser/ui/browser.cc b/chrome/browser/ui/browser.cc
index 9603137595182b9f442bd54cd6711fffc1fe7af8..ef7b235234126d511921233b1579442c4d9598db 100644
--- a/chrome/browser/ui/browser.cc
+++ b/chrome/browser/ui/browser.cc
@@ -45,6 +45,7 @@
 #include "chrome/browser/background/background_contents_service_factory.h"
 #include "chrome/browser/bookmarks/bookmark_model_factory.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/buildflags.h"
 #include "chrome/browser/content_settings/host_content_settings_map_factory.h"
 #include "chrome/browser/content_settings/mixed_content_settings_tab_helper.h"
@@ -623,11 +624,17 @@ Browser::Browser(const CreateParams& params)
 
   tab_strip_model_->AddObserver(this);
 
+  browseros::SyncShowTabGroupsInBookmarkBarPref(profile_->GetPrefs());
+
   profile_pref_registrar_.Init(profile_->GetPrefs());
   profile_pref_registrar_.Add(
       prefs::kDevToolsAvailability,
       base::BindRepeating(&Browser::OnDevToolsAvailabilityChanged,
                           base::Unretained(this)));
+  profile_pref_registrar_.Add(
+      browseros::prefs::kShowTabGroupsInBookmarkBar,
+      base::BindRepeating(&browseros::ApplyShowTabGroupsInBookmarkBarPref,
+                          base::Unretained(profile_->GetPrefs())));
 
   ProfileMetrics::LogProfileLaunch(profile_);
 
@@ -2287,6 +2294,11 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
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
@@ -2299,15 +2311,18 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
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
