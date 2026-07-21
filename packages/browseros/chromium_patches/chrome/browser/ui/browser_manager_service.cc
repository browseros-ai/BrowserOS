diff --git a/chrome/browser/ui/browser_manager_service.cc b/chrome/browser/ui/browser_manager_service.cc
index 3a9c66792158d0a7baba089287029a5eae346544..7c8857439f44136c3d0bc604ae56a133beadf87c 100644
--- a/chrome/browser/ui/browser_manager_service.cc
+++ b/chrome/browser/ui/browser_manager_service.cc
@@ -231,6 +231,12 @@ BrowserCollection::BrowserVector BrowserManagerService::GetBrowsers(
 
 void BrowserManagerService::OnBrowserActivated(
     BrowserWindowInterface* browser) {
+  // Hidden Browsers never become last-active — GetLastActiveBrowser() and
+  // activation-ordered iteration should always target user-visible windows.
+  if (browser->GetBrowserForMigrationOnly()->is_hidden()) {
+    return;
+  }
+
   // Move `browser` to the front of the activation list.
   auto it = std::ranges::find(browsers_activation_order_, browser);
   CHECK(it != browsers_activation_order_.end());
