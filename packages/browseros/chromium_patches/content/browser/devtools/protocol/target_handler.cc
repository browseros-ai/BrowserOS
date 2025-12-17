diff --git a/content/browser/devtools/protocol/target_handler.cc b/content/browser/devtools/protocol/target_handler.cc
index b1721338d7523..b63d11cb5fc2e 100644
--- a/content/browser/devtools/protocol/target_handler.cc
+++ b/content/browser/devtools/protocol/target_handler.cc
@@ -1411,11 +1411,11 @@ void TargetHandler::DevToolsAgentHostDestroyed(DevToolsAgentHost* host) {
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
