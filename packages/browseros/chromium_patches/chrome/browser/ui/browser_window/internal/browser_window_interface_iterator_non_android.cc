diff --git a/chrome/browser/ui/browser_window/internal/browser_window_interface_iterator_non_android.cc b/chrome/browser/ui/browser_window/internal/browser_window_interface_iterator_non_android.cc
index 325fc1a4b3dd2aa77ba5bf22a4e00afe2310c663..ad59be99d7e3e0b5389c34e52282cf13bb242c0f 100644
--- a/chrome/browser/ui/browser_window/internal/browser_window_interface_iterator_non_android.cc
+++ b/chrome/browser/ui/browser_window/internal/browser_window_interface_iterator_non_android.cc
@@ -3,6 +3,8 @@
 // found in the LICENSE file.
 
 #include "base/functional/function_ref.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h"
 #include "chrome/browser/ui/browser_window/public/global_browser_collection.h"
 
@@ -43,3 +45,7 @@ BrowserWindowInterface* GetLastActiveBrowserWindowInterfaceWithAnyProfile() {
       });
   return last_active;
 }
+
+bool ShouldShowBrowserInUserInterface(BrowserWindowInterface* browser) {
+  return browser && !browser->GetBrowserForMigrationOnly()->is_hidden();
+}
