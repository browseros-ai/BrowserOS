diff --git a/content/browser/devtools/protocol/target_handler.cc b/content/browser/devtools/protocol/target_handler.cc
index f051aaf1c55ed686d6652c4b32622532f0692487..6ae6a672b7e121ee82b949b4bf9abeec061bfd92 100644
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
 
@@ -461,10 +474,11 @@ class TargetHandler::RequestThrottle : public TargetHandler::Throttle {
 
 class TargetHandler::Session : public DevToolsAgentHostClient {
  public:
-  static std::optional<std::string> Attach(TargetHandler* handler,
-                                           scoped_refptr<DevToolsAgentHost> agent_host,
-                                           bool waiting_for_debugger,
-                                           bool flatten_protocol) {
+  static std::optional<std::string> Attach(
+      TargetHandler* handler,
+      scoped_refptr<DevToolsAgentHost> agent_host,
+      bool waiting_for_debugger,
+      bool flatten_protocol) {
     std::string id = base::UnguessableToken::Create().ToString();
     // We don't support or allow the non-flattened protocol when in binary mode.
     // So, we coerce the setting to true, as the non-flattened mode is
@@ -1494,11 +1508,11 @@ void TargetHandler::DevToolsAgentHostDestroyed(DevToolsAgentHost* host) {
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
