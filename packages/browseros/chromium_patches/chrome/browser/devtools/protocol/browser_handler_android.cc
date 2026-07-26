diff --git a/chrome/browser/devtools/protocol/browser_handler_android.cc b/chrome/browser/devtools/protocol/browser_handler_android.cc
index 82199c6e2e93b..7978474dda89b 100644
--- a/chrome/browser/devtools/protocol/browser_handler_android.cc
+++ b/chrome/browser/devtools/protocol/browser_handler_android.cc
@@ -11,6 +11,7 @@
 #include "chrome/browser/android/tab_android.h"
 #include "chrome/browser/ui/android/tab_model/tab_model.h"
 #include "chrome/browser/ui/android/tab_model/tab_model_list.h"
+#include "content/public/browser/devtools_agent_host.h"
 
 using protocol::Response;
 
@@ -55,6 +56,59 @@ Response BrowserHandlerAndroid::GetWindowForTarget(
   return Response::ServerError("Browser window not found");
 }
 
+Response BrowserHandlerAndroid::GetTabForTarget(
+    std::optional<std::string> target_id,
+    int* out_tab_id,
+    int* out_window_id) {
+  auto host =
+      content::DevToolsAgentHost::GetForId(target_id.value_or(target_id_));
+  if (!host)
+    return Response::ServerError("No matching target");
+  content::WebContents* web_contents = host->GetWebContents();
+  if (!web_contents)
+    return Response::ServerError("No web contents in the target");
+
+  for (TabModel* model : TabModelList::models()) {
+    for (int i = 0; i < model->GetTabCount(); ++i) {
+      TabAndroid* tab = model->GetTabAt(i);
+      if (tab->web_contents() == web_contents) {
+        *out_tab_id = tab->GetAndroidId();
+        *out_window_id = tab->GetWindowId().id();
+        return Response::Success();
+      }
+    }
+  }
+
+  return Response::ServerError("Tab not found");
+}
+
+Response BrowserHandlerAndroid::GetTargetForTab(
+    int tab_id,
+    std::string* out_target_id,
+    int* out_window_id) {
+  for (TabModel* model : TabModelList::models()) {
+    for (int i = 0; i < model->GetTabCount(); ++i) {
+      TabAndroid* tab = model->GetTabAt(i);
+      if (tab->GetAndroidId() == tab_id) {
+        content::WebContents* web_contents = tab->web_contents();
+        if (!web_contents)
+          return Response::ServerError("Tab has no web contents");
+
+        scoped_refptr<content::DevToolsAgentHost> host =
+            content::DevToolsAgentHost::GetOrCreateFor(web_contents);
+        if (!host)
+          return Response::ServerError("No target for tab");
+
+        *out_target_id = host->GetId();
+        *out_window_id = tab->GetWindowId().id();
+        return Response::Success();
+      }
+    }
+  }
+
+  return Response::ServerError("No tab with given id");
+}
+
 Response BrowserHandlerAndroid::GetWindowBounds(
     int window_id,
     std::unique_ptr<protocol::Browser::Bounds>* out_bounds) {
@@ -92,3 +146,178 @@ protocol::Response BrowserHandlerAndroid::AddPrivacySandboxEnrollmentOverride(
     const std::string& in_url) {
   return Response::MethodNotFound(kNotImplemented);
 }
+
+// --- Window Management (all stubs) ---
+
+Response BrowserHandlerAndroid::GetWindows(
+    std::unique_ptr<protocol::Array<protocol::Browser::WindowInfo>>*
+        out_windows) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::GetActiveWindow(
+    std::unique_ptr<protocol::Browser::WindowInfo>* out_window) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::CreateWindow(
+    std::optional<std::string> url,
+    std::unique_ptr<protocol::Browser::Bounds> bounds,
+    std::optional<std::string> window_type,
+    std::optional<bool> hidden,
+    std::optional<std::string> browser_context_id,
+    std::unique_ptr<protocol::Browser::WindowInfo>* out_window) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::CloseWindow(int window_id) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::ActivateWindow(int window_id) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::SetWindowVisibility(
+    int window_id,
+    bool visible,
+    std::optional<bool> activate,
+    std::unique_ptr<protocol::Browser::WindowInfo>* out_window,
+    bool* out_replaced,
+    int* out_previous_window_id) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+// --- Tab Management ---
+
+Response BrowserHandlerAndroid::GetTabs(
+    std::optional<int> window_id,
+    std::optional<bool> include_hidden,
+    std::unique_ptr<protocol::Array<protocol::Browser::TabInfo>>* out_tabs) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::GetActiveTab(
+    std::optional<int> window_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::GetTabInfo(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::CreateTab(
+    std::optional<std::string> url,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::optional<bool> background,
+    std::optional<bool> pinned,
+    std::optional<std::string> browser_context_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::CloseTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::ActivateTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::MoveTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::DuplicateTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::PinTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::UnpinTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::ShowTab(
+    std::optional<std::string> target_id,
+    std::optional<int> tab_id,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::optional<bool> activate,
+    std::unique_ptr<protocol::Browser::TabInfo>* out_tab) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+// --- Tab Group Management (all stubs) ---
+
+Response BrowserHandlerAndroid::GetTabGroups(
+    std::optional<int> window_id,
+    std::unique_ptr<protocol::Array<protocol::Browser::TabGroupInfo>>*
+        out_groups) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::CreateTabGroup(
+    std::unique_ptr<protocol::Array<int>> tab_ids,
+    std::optional<std::string> title,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::UpdateTabGroup(
+    const std::string& group_id,
+    std::optional<std::string> title,
+    std::optional<std::string> color,
+    std::optional<bool> collapsed,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::CloseTabGroup(const std::string& group_id) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::AddTabsToGroup(
+    const std::string& group_id,
+    std::unique_ptr<protocol::Array<int>> tab_ids,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::RemoveTabsFromGroup(
+    std::unique_ptr<protocol::Array<int>> tab_ids) {
+  return Response::MethodNotFound(kNotImplemented);
+}
+
+Response BrowserHandlerAndroid::MoveTabGroup(
+    const std::string& group_id,
+    std::optional<int> window_id,
+    std::optional<int> index,
+    std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) {
+  return Response::MethodNotFound(kNotImplemented);
+}
