diff --git a/chrome/browser/extensions/chrome_extension_registrar_delegate.cc b/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
index ed0e71e24734cd358ed19750872ed1ea0942a6eb..c78cb415182b6d1503a4837daccdd89e100008a9 100644
--- a/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
+++ b/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
@@ -13,6 +13,7 @@
 #include "base/metrics/histogram_functions.h"
 #include "base/metrics/histogram_macros.h"
 #include "base/notimplemented.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/component_loader.h"
 #include "chrome/browser/extensions/corrupted_extension_reinstaller.h"
 #include "chrome/browser/extensions/data_deleter.h"
@@ -258,7 +259,17 @@ void ChromeExtensionRegistrarDelegate::PostUninstallExtension(
     }
   }
 
-  DataDeleter::StartDeleting(profile_, extension.get(), subtask_done_callback);
+  // Preserve chrome.storage.local data for BrowserOS extensions. These may be
+  // transiently uninstalled during update cycles (e.g., when both bundled CRX
+  // and remote config fail on startup). User configuration must survive.
+  if (browseros::IsKnownBrowserOSExtension(extension->id())) {
+    LOG(INFO) << "browseros: Preserving storage for extension "
+              << extension->id();
+    subtask_done_callback.Run();
+  } else {
+    DataDeleter::StartDeleting(profile_, extension.get(),
+                               subtask_done_callback);
+  }
 }
 
 void ChromeExtensionRegistrarDelegate::DoLoadExtensionForReload(
@@ -324,6 +335,13 @@ bool ChromeExtensionRegistrarDelegate::CanDisableExtension(
     return true;
   }
 
+  // - BrowserOS extensions cannot be disabled by users
+  if (browseros::IsActiveBrowserOSExtension(extension->id())) {
+    LOG(INFO) << "browseros: Extension " << extension->id()
+              << " cannot be disabled (BrowserOS extension)";
+    return false;
+  }
+
   // - Shared modules are just resources used by other extensions, and are not
   //   user-controlled.
   if (SharedModuleInfo::IsSharedModule(extension)) {
