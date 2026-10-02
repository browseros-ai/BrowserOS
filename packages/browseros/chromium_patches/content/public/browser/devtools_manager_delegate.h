diff --git a/content/public/browser/devtools_manager_delegate.h b/content/public/browser/devtools_manager_delegate.h
index 069a7b48a71ed650b2088721e61fc7d6d7947c08..dbe928914713f552eddc01dde051cf3f17a082d6 100644
--- a/content/public/browser/devtools_manager_delegate.h
+++ b/content/public/browser/devtools_manager_delegate.h
@@ -101,6 +101,14 @@ class CONTENT_EXPORT DevToolsManagerDelegate {
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
