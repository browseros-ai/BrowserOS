diff --git a/content/browser/devtools/protocol/target_handler.cc b/content/browser/devtools/protocol/target_handler.cc
index f051aaf1c55ed686d6652c4b32622532f0692487..454cc899cb6f25e1c51e5d68120e08a1756d036f 100644
--- a/content/browser/devtools/protocol/target_handler.cc
+++ b/content/browser/devtools/protocol/target_handler.cc
@@ -135,6 +135,19 @@ std::unique_ptr<Target::TargetInfo> BuildTargetInfo(
       }
     }
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
 
@@ -1494,11 +1507,11 @@ void TargetHandler::DevToolsAgentHostDestroyed(DevToolsAgentHost* host) {
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
