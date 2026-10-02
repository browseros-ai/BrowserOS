diff --git a/chrome/browser/extensions/chrome_extension_registrar_delegate.cc b/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
index 7ff5fb309529d5a776f0fdb29e07ef1e001f06f4..276eb347ba2d61fca627ad34b68b28a454b1f0b9 100644
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
