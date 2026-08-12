diff --git a/content/public/browser/devtools_manager_delegate.cc b/content/public/browser/devtools_manager_delegate.cc
index 8257dd01a79e617ff12554fd6cf5d32dc1abaf50..b01dc69a65c4f03ad3f0edcf2eb1229840e03933 100644
--- a/content/public/browser/devtools_manager_delegate.cc
+++ b/content/public/browser/devtools_manager_delegate.cc
@@ -62,6 +62,12 @@ std::optional<bool> DevToolsManagerDelegate::ShouldReportAsTabTarget(
   return std::nullopt;
 }
 
+bool DevToolsManagerDelegate::GetTargetTabId(WebContents* web_contents,
+                                              int* tab_id,
+                                              int* window_id) {
+  return false;
+}
+
 DevToolsAgentHost::List DevToolsManagerDelegate::RemoteDebuggingTargets(
     DevToolsManagerDelegate::TargetType target_type) {
   return DevToolsAgentHost::GetOrCreateAll();
