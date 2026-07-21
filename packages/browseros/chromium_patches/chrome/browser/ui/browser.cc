diff --git a/chrome/browser/ui/browser.cc b/chrome/browser/ui/browser.cc
index 0777bf71430c810f7b6ae41a7708cdd069b6c5a4..efd685b8c6bf4aa82990019c328c98d042668824 100644
--- a/chrome/browser/ui/browser.cc
+++ b/chrome/browser/ui/browser.cc
@@ -47,6 +47,7 @@
 #include "chrome/browser/background/background_contents_service_factory.h"
 #include "chrome/browser/bookmarks/bookmark_model_factory.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/buildflags.h"
 #include "chrome/browser/content_settings/host_content_settings_map_factory.h"
 #include "chrome/browser/content_settings/mixed_content_settings_tab_helper.h"
@@ -530,6 +531,7 @@ Browser::Browser(const CreateParams& params)
       is_trusted_source_(params.trusted_source),
       session_id_(SessionID::NewUnique()),
       omit_from_session_restore_(params.omit_from_session_restore),
+      is_hidden_(params.hidden),
       should_trigger_session_restore_(params.should_trigger_session_restore),
       cancel_download_confirmation_state_(
           CancelDownloadConfirmationState::kNotPrompted),
@@ -555,11 +557,17 @@ Browser::Browser(const CreateParams& params)
 
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
 
@@ -627,6 +635,10 @@ Browser::Browser(const CreateParams& params)
 }
 
 Browser::~Browser() {
+  // Release capturer-count pins early so renderers see Visibility updates
+  // before WebContents is torn down.
+  hidden_tab_pins_.clear();
+
   if (!is_delete_scheduled_) {
     // Guarantee the Browser has performed the necessary cleanup in the
     // `OnWindowClosing()` lifecycle hook. This may not be invoked during
@@ -1203,6 +1215,19 @@ WebContents* Browser::OpenURL(
 ///////////////////////////////////////////////////////////////////////////////
 // Browser, TabStripModelObserver implementation:
 
+void Browser::PinHiddenTabVisibility(content::WebContents* web_contents) {
+  if (!web_contents || hidden_tab_pins_.contains(web_contents)) {
+    return;
+  }
+  hidden_tab_pins_[web_contents] = web_contents->IncrementCapturerCount(
+      gfx::Size(), /*stay_hidden=*/false, /*stay_awake=*/true,
+      /*is_activity=*/true);
+}
+
+void Browser::UnpinHiddenTabVisibility(content::WebContents* web_contents) {
+  hidden_tab_pins_.erase(web_contents);
+}
+
 void Browser::OnTabStripModelChanged(TabStripModel* tab_strip_model,
                                      const TabStripModelChange& change,
                                      const TabStripSelectionChange& selection) {
@@ -1220,6 +1245,9 @@ void Browser::OnTabStripModelChanged(TabStripModel* tab_strip_model,
       }
       for (const auto& contents : change.GetInsert()->contents) {
         OnTabInsertedAt(contents.contents, contents.index);
+        if (is_hidden_) {
+          PinHiddenTabVisibility(contents.contents);
+        }
       }
       break;
     }
@@ -1231,6 +1259,9 @@ void Browser::OnTabStripModelChanged(TabStripModel* tab_strip_model,
         }
         OnTabDetached(contents.tab, contents.contents == selection.old_contents,
                       had_active_modal_dialog);
+        if (is_hidden_) {
+          UnpinHiddenTabVisibility(contents.contents);
+        }
       }
       break;
     }
@@ -1240,6 +1271,10 @@ void Browser::OnTabStripModelChanged(TabStripModel* tab_strip_model,
       auto* replace = change.GetReplace();
       OnTabReplacedAt(replace->old_contents, replace->new_contents,
                       replace->index);
+      if (is_hidden_) {
+        UnpinHiddenTabVisibility(replace->old_contents);
+        PinHiddenTabVisibility(replace->new_contents);
+      }
       break;
     }
     case TabStripModelChange::kSelectionOnly:
@@ -1824,6 +1859,11 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
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
@@ -1836,15 +1876,18 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
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
