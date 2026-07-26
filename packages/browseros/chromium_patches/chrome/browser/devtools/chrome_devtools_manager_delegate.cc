diff --git a/chrome/browser/devtools/chrome_devtools_manager_delegate.cc b/chrome/browser/devtools/chrome_devtools_manager_delegate.cc
index af831528b7b04..79756c4a7a54b 100644
--- a/chrome/browser/devtools/chrome_devtools_manager_delegate.cc
+++ b/chrome/browser/devtools/chrome_devtools_manager_delegate.cc
@@ -45,6 +45,7 @@
 #include "components/guest_view/browser/guest_view_base.h"
 #include "components/keep_alive_registry/keep_alive_types.h"
 #include "components/keep_alive_registry/scoped_keep_alive.h"
+#include "components/sessions/content/session_tab_helper.h"
 #include "components/tabs/public/tab_interface.h"
 #include "content/public/browser/browser_thread.h"
 #include "content/public/browser/devtools_agent_host.h"
@@ -335,6 +336,20 @@ std::optional<bool> ChromeDevToolsManagerDelegate::ShouldReportAsTabTarget(
   return std::nullopt;
 }
 
+bool ChromeDevToolsManagerDelegate::GetTargetTabId(
+    content::WebContents* web_contents,
+    int* tab_id,
+    int* window_id) {
+  SessionID sid = sessions::SessionTabHelper::IdForTab(web_contents);
+  if (!sid.is_valid())
+    return false;
+  *tab_id = sid.id();
+  SessionID wid =
+      sessions::SessionTabHelper::IdForWindowContainingTab(web_contents);
+  *window_id = wid.is_valid() ? wid.id() : -1;
+  return true;
+}
+
 std::string ChromeDevToolsManagerDelegate::GetTargetTitle(
     content::WebContents* web_contents) {
   if (auto iwa_name_version = GetIsolatedWebAppNameAndVersion(web_contents)) {
