diff --git a/chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h b/chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h
index 6f6d87a62486e6d5c905c15b19e0d2b10bcbe014..68317708a8c31b1f450ca285a8701ca7779b17f4 100644
--- a/chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h
+++ b/chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h
@@ -84,4 +84,10 @@ void ForEachCurrentAndNewBrowserWindowInterfaceOrderedByActivation(
 // chrome/browser/ui/browser_window/public/global_browser_collection.h instead.
 BrowserWindowInterface* GetLastActiveBrowserWindowInterfaceWithAnyProfile();
 
+// True if `browser` should appear in user-facing UI enumerations (tab search,
+// window menus, drag-drop candidates, extensions API, etc.). Returns false for
+// hidden Browsers — agent-owned workspaces that exist in the browser
+// collections but are not part of the user's visible windowing experience.
+bool ShouldShowBrowserInUserInterface(BrowserWindowInterface* browser);
+
 #endif  // CHROME_BROWSER_UI_BROWSER_WINDOW_PUBLIC_BROWSER_WINDOW_INTERFACE_ITERATOR_H_
