diff --git a/chrome/browser/devtools/chrome_devtools_manager_delegate.cc b/chrome/browser/devtools/chrome_devtools_manager_delegate.cc
index c9f49cd198cd29a4f4ad3c2321c9b7a874b5e899..220a88e6f2712ff409340f492920402a986d782c 100644
--- a/chrome/browser/devtools/chrome_devtools_manager_delegate.cc
+++ b/chrome/browser/devtools/chrome_devtools_manager_delegate.cc
@@ -48,6 +48,7 @@
 #include "components/guest_view/browser/guest_view_base.h"
 #include "components/keep_alive_registry/keep_alive_types.h"
 #include "components/keep_alive_registry/scoped_keep_alive.h"
+#include "components/sessions/content/session_tab_helper.h"
 #include "components/tabs/public/tab_interface.h"
 #include "content/public/browser/browser_thread.h"
 #include "content/public/browser/devtools_agent_host.h"
@@ -343,6 +344,20 @@ std::optional<bool> ChromeDevToolsManagerDelegate::ShouldReportAsTabTarget(
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
