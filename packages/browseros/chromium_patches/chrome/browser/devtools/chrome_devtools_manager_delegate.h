diff --git a/chrome/browser/devtools/chrome_devtools_manager_delegate.h b/chrome/browser/devtools/chrome_devtools_manager_delegate.h
index a48b3a9ebdf6f323a636dafbaf99532a2f375ce4..8fab05a9d346d1ccd194093c9fac45723ddfb980 100644
--- a/chrome/browser/devtools/chrome_devtools_manager_delegate.h
+++ b/chrome/browser/devtools/chrome_devtools_manager_delegate.h
@@ -80,6 +80,9 @@ class ChromeDevToolsManagerDelegate : public content::DevToolsManagerDelegate,
       content::DevToolsAgentHost* agent_host) override;
   std::optional<bool> ShouldReportAsTabTarget(
       content::WebContents* web_contents) override;
+  bool GetTargetTabId(content::WebContents* web_contents,
+                      int* tab_id,
+                      int* window_id) override;
 
   content::BrowserContext* CreateBrowserContext() override;
   void DisposeBrowserContext(content::BrowserContext*,
