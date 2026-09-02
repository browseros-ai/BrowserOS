diff --git a/chrome/browser/ui/browser.cc b/chrome/browser/ui/browser.cc
index 0777bf71430c810f7b6ae41a7708cdd069b6c5a4..5b9c9d1d371ff49436c0deedfa031d18f53192cc 100644
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
@@ -331,6 +332,22 @@ constexpr base::TimeDelta kUIUpdateCoalescingTime = base::Milliseconds(200);
 BASE_FEATURE(kBackgroundActorTaskPopupsOpenInBackground,
              base::FEATURE_ENABLED_BY_DEFAULT);
 
+// BrowserOS: whether |contents| is an automation-driven tab whose activity must
+// never pull the user's attention; see
+// browseros::prefs::kAutomationNeverStealsFocus. "Automation-driven" is
+// approximated by a DevTools client being attached,
+// which covers every tab the claw-server or any other CDP client acts on. The
+// claw-server never detaches its page sessions, so this stays true for a tab an
+// agent touched earlier in the browser session even after the agent is done.
+// That is accepted for now because every such tab was opened by an agent; if a
+// human-owned tab ever needs to shed the flag, detach on ownership release or
+// switch to an explicit per-tab marker.
+bool ShouldSuppressAutomationFocus(Profile* profile, WebContents* contents) {
+  return contents && profile &&
+         browseros::AutomationNeverStealsFocus(profile->GetPrefs()) &&
+         content::DevToolsAgentHost::IsDebuggerAttached(contents);
+}
+
 const extensions::Extension* GetExtensionForOrigin(
     Profile* profile,
     const GURL& security_origin) {
@@ -555,11 +572,17 @@ Browser::Browser(const CreateParams& params)
 
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
 
@@ -1684,6 +1707,24 @@ content::WebContents* Browser::AddNewContents(
     }
   }
 
+  // BrowserOS: the same treatment for automation-driven tabs. A click an agent
+  // synthesized over CDP is a trusted gesture, so a target=_blank link would
+  // open an active tab and window.open would raise a popup window. When the
+  // source tab is not the one the user is looking at, keep both in the
+  // background. As with the actor gate above, an active source tab means the
+  // user is watching and normal behaviour applies.
+  if (source && ShouldSuppressAutomationFocus(profile_, source)) {
+    tabs::TabInterface* source_tab =
+        tabs::TabInterface::MaybeGetFromContents(source);
+    if (!source_tab || !source_tab->IsActivated()) {
+      if (disposition == WindowOpenDisposition::NEW_POPUP) {
+        window_action = NavigateParams::WindowAction::kShowWindowInactive;
+      } else if (disposition == WindowOpenDisposition::NEW_FOREGROUND_TAB) {
+        disposition = WindowOpenDisposition::NEW_BACKGROUND_TAB;
+      }
+    }
+  }
+
   return chrome::AddWebContents(this, source, std::move(new_contents),
                                 target_url, disposition, window_features,
                                 window_action, user_gesture);
@@ -1696,6 +1737,13 @@ void Browser::ActivateContents(WebContents* contents) {
   if (index == TabStripModel::kNoTab) {
     return;
   }
+  // BrowserOS: this is the single funnel for renderer- and CDP-initiated
+  // activation (Page.bringToFront, Target.activateTarget, a popup's
+  // window.focus(), fullscreen requests). An automation-driven tab must not
+  // switch the user's tab or raise the window through it.
+  if (ShouldSuppressAutomationFocus(profile_, contents)) {
+    return;
+  }
   tab_strip_model_->ActivateTabAt(index);
   window_->Activate();
 }
@@ -1824,6 +1872,11 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
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
@@ -1836,15 +1889,18 @@ bool Browser::ShouldFocusLocationBarByDefault(WebContents* source) {
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
