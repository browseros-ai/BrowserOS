diff --git a/chrome/browser/ui/browser_web_contents_delegate/browser_web_contents_delegate.cc b/chrome/browser/ui/browser_web_contents_delegate/browser_web_contents_delegate.cc
index d3fa7eea6db807d9a9b3467da4f468c555468b26..06847a35125ce4936598892e941ef5f07027f9c1 100644
--- a/chrome/browser/ui/browser_web_contents_delegate/browser_web_contents_delegate.cc
+++ b/chrome/browser/ui/browser_web_contents_delegate/browser_web_contents_delegate.cc
@@ -13,6 +13,7 @@
 #include "chrome/browser/background/background_contents.h"
 #include "chrome/browser/background/background_contents_service.h"
 #include "chrome/browser/background/background_contents_service_factory.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/content_settings/host_content_settings_map_factory.h"
 #include "chrome/browser/content_settings/page_specific_content_settings_delegate.h"
 #include "chrome/browser/custom_handlers/protocol_handler_registry_factory.h"
@@ -74,6 +75,7 @@
 #include "components/split_tabs/split_tab_id.h"
 #include "components/tabs/public/split_tab_data.h"
 #include "components/tabs/public/tab_interface.h"
+#include "content/public/browser/devtools_agent_host.h"
 #include "content/public/browser/keyboard_event_processing_result.h"
 #include "content/public/browser/navigation_controller.h"
 #include "content/public/browser/navigation_entry.h"
@@ -129,6 +131,18 @@ DEFINE_USER_DATA(BrowserWebContentsDelegate);
 
 namespace {
 
+// BrowserOS treats an attached DevTools client as the automation marker.
+// The delegate is the shared renderer/CDP activation boundary, so the
+// profile preference must be checked here before a tab or window is raised.
+// Agent page sessions stay attached; touched tabs retain this behavior until
+// detachment, including tabs attached by a human DevTools client.
+bool ShouldSuppressAutomationFocus(Profile* profile,
+                                   content::WebContents* contents) {
+  return contents && profile &&
+         browseros::AutomationNeverStealsFocus(profile->GetPrefs()) &&
+         content::DevToolsAgentHost::IsDebuggerAttached(contents);
+}
+
 const extensions::Extension* GetExtensionForOrigin(
     Profile* profile,
     const GURL& security_origin) {
@@ -738,6 +752,21 @@ content::WebContents* BrowserWebContentsDelegate::AddNewContents(
     }
   }
 
+  // CDP clicks count as trusted gestures. Keep popups and new tabs from
+  // background automation in the background, but preserve normal popup
+  // behavior when the user is watching the source tab.
+  if (ShouldSuppressAutomationFocus(browser_->GetProfile(), source)) {
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
   return chrome::AddWebContents(&*browser_, source, std::move(new_contents),
                                 target_url, disposition, window_features,
                                 window_action, user_gesture);
@@ -751,6 +780,11 @@ void BrowserWebContentsDelegate::ActivateContents(
   if (index == TabStripModel::kNoTab) {
     return;
   }
+  // Renderer/CDP activation must not switch tabs or raise the window for
+  // automation. User-driven tab selection does not pass through this gate.
+  if (ShouldSuppressAutomationFocus(browser_->GetProfile(), contents)) {
+    return;
+  }
   browser_->GetTabStripModel()->ActivateTabAt(index);
   window_->Activate();
 }
@@ -892,6 +926,10 @@ bool BrowserWebContentsDelegate::ShouldFocusLocationBarByDefault(
       source->GetController().GetPendingEntry()
           ? source->GetController().GetPendingEntry()
           : source->GetController().GetLastCommittedEntry();
+  // BrowserOS can give the NTP content initial focus. Apply the same
+  // preference to real, virtual, split-view and Instant NTP entries.
+  const bool ntp_focus_content =
+      browseros::IsNtpFocusContentEnabled(browser_->GetProfile()->GetPrefs());
   if (entry) {
     const GURL& url = entry->GetURL();
     const GURL& virtual_url = entry->GetVirtualURL();
@@ -904,15 +942,15 @@ bool BrowserWebContentsDelegate::ShouldFocusLocationBarByDefault(
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
+  return search::NavEntryIsInstantNTP(source, entry) && !ntp_focus_content;
 }
 
 bool BrowserWebContentsDelegate::ShouldFocusPageAfterCrash(
