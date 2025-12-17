diff --git a/chrome/browser/extensions/chrome_extension_registrar_delegate.cc b/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
index 6eec0585e8925..55c2a73647527 100644
--- a/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
+++ b/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
@@ -12,6 +12,7 @@
 #include "base/metrics/histogram_functions.h"
 #include "base/metrics/histogram_macros.h"
 #include "base/notimplemented.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/component_loader.h"
 #include "chrome/browser/extensions/corrupted_extension_reinstaller.h"
 #include "chrome/browser/extensions/data_deleter.h"
@@ -317,6 +318,13 @@ bool ChromeExtensionRegistrarDelegate::CanDisableExtension(
     return true;
   }
 
+  // - BrowserOS extensions cannot be disabled by users
+  if (browseros::IsBrowserOSExtension(extension->id())) {
+    LOG(INFO) << "browseros: Extension " << extension->id()
+              << " cannot be disabled (BrowserOS extension)";
+    return false;
+  }
+
   // - Shared modules are just resources used by other extensions, and are not
   //   user-controlled.
   if (SharedModuleInfo::IsSharedModule(extension)) {
