diff --git a/chrome/browser/ui/browser_window/internal/profile_browser_collection.cc b/chrome/browser/ui/browser_window/internal/profile_browser_collection.cc
index 5329d73ade213228cfd01bb8be971b252d458f6b..58dd7d99df2a9944e0410de1fce8a6ed81daafea 100644
--- a/chrome/browser/ui/browser_window/internal/profile_browser_collection.cc
+++ b/chrome/browser/ui/browser_window/internal/profile_browser_collection.cc
@@ -41,6 +41,12 @@ BrowserWindowInterface* ProfileBrowserCollection::FindTabbedBrowser(
     }
 
 #if !BUILDFLAG(IS_ANDROID)
+    // Hidden Browsers are agent-owned scratch space; never pick them as a
+    // default target for user-initiated actions (new tabs, find-any, etc.).
+    if (browser->GetBrowserForMigrationOnly()->is_hidden()) {
+      return true;
+    }
+
     BrowserWindow* browser_window = BrowserWindow::FromBrowser(browser);
     if (!browser_window || !browser_window->IsOnCurrentWorkspace()) {
       return true;
