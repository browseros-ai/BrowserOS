diff --git a/content/browser/devtools/protocol/target_handler.cc b/content/browser/devtools/protocol/target_handler.cc
index e77c87427f88569d10830c31e665cf93f6e8dce3..a9a3db3622d24a4c3e2f711db05badf54b3cdd7a 100644
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
 
@@ -459,10 +472,11 @@ class TargetHandler::RequestThrottle : public TargetHandler::Throttle {
 
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
@@ -1507,11 +1521,11 @@ void TargetHandler::DevToolsAgentHostDestroyed(DevToolsAgentHost* host) {
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
