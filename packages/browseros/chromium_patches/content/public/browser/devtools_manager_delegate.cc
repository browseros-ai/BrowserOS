diff --git a/content/public/browser/devtools_manager_delegate.cc b/content/public/browser/devtools_manager_delegate.cc
index 02e70247e20fd..098273cc00bdd 100644
--- a/content/public/browser/devtools_manager_delegate.cc
+++ b/content/public/browser/devtools_manager_delegate.cc
@@ -57,6 +57,12 @@ std::optional<bool> DevToolsManagerDelegate::ShouldReportAsTabTarget(
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
