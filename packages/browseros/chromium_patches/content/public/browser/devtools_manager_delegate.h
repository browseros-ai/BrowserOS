diff --git a/content/public/browser/devtools_manager_delegate.h b/content/public/browser/devtools_manager_delegate.h
index f08399ea042217b4c72d0a5cb4d71067ced65a87..d27be9caa801a0e927f2b250a728eb22aada2db6 100644
--- a/content/public/browser/devtools_manager_delegate.h
+++ b/content/public/browser/devtools_manager_delegate.h
@@ -100,6 +100,14 @@ class CONTENT_EXPORT DevToolsManagerDelegate {
   virtual std::optional<bool> ShouldReportAsTabTarget(
       WebContents* web_contents);
 
+  // Returns session-scoped tab and window identifiers for the given
+  // |web_contents|. Embedders that support tab identity (e.g. Chrome)
+  // should override this to populate tabId/windowId in TargetInfo.
+  // Returns false if the web contents does not have tab identity.
+  virtual bool GetTargetTabId(WebContents* web_contents,
+                              int* tab_id,
+                              int* window_id);
+
   // Chrome Devtools Protocol Target type to use. Before MPArch frame targets
   // were used, which correspond to the primary outermost frame in the
   // WebContents. With prerender and other MPArch features, there could be
