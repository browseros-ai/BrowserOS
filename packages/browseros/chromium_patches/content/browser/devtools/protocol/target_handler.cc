diff --git a/content/browser/devtools/protocol/target_handler.cc b/content/browser/devtools/protocol/target_handler.cc
index 3732a63380831..59a3948f998f9 100644
--- a/content/browser/devtools/protocol/target_handler.cc
+++ b/content/browser/devtools/protocol/target_handler.cc
@@ -120,6 +120,19 @@ std::unique_ptr<Target::TargetInfo> BuildTargetInfo(
   if (!subtype.empty()) {
     target_info->SetSubtype(subtype);
   }
+  WebContents* web_contents = host->GetWebContents();
+  if (web_contents) {
+    DevToolsManagerDelegate* delegate =
+        DevToolsManager::GetInstance()->delegate();
+    int tab_id, window_id;
+    if (delegate &&
+        delegate->GetTargetTabId(web_contents, &tab_id, &window_id)) {
+      target_info->SetTabId(tab_id);
+      if (window_id >= 0) {
+        target_info->SetWindowId(window_id);
+      }
+    }
+  }
   return target_info;
 }
 
@@ -1437,11 +1450,11 @@ void TargetHandler::DevToolsAgentHostDestroyed(DevToolsAgentHost* host) {
 }
 
 void TargetHandler::DevToolsAgentHostAttached(DevToolsAgentHost* host) {
-  TargetInfoChanged(host);
+  // TargetInfoChanged(host);
 }
 
 void TargetHandler::DevToolsAgentHostDetached(DevToolsAgentHost* host) {
-  TargetInfoChanged(host);
+  // TargetInfoChanged(host);
 }
 
 void TargetHandler::DevToolsAgentHostCrashed(DevToolsAgentHost* host,
