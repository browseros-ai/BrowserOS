diff --git a/chrome/browser/extensions/chrome_extension_registrar_delegate.cc b/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
index a9fc73b11371d..41f81b3712a6c 100644
--- a/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
+++ b/chrome/browser/extensions/chrome_extension_registrar_delegate.cc
@@ -12,6 +12,7 @@
 #include "base/metrics/histogram_functions.h"
 #include "base/metrics/histogram_macros.h"
 #include "base/notimplemented.h"
+#include "chrome/browser/extensions/blockbrowser_extension_constants.h"
 #include "chrome/browser/extensions/component_loader.h"
 #include "chrome/browser/extensions/corrupted_extension_reinstaller.h"
 #include "chrome/browser/extensions/data_deleter.h"
@@ -314,6 +315,15 @@ bool ChromeExtensionRegistrarDelegate::CanDisableExtension(
     return true;
   }
 
+  // - BlockBrowser extensions cannot be disabled by users
+  for (const char* allowed_id : browseros::kAllowedExtensions) {
+    if (extension->id() == allowed_id) {
+      LOG(INFO) << "browseros: Extension " << extension->id() 
+                << " cannot be disabled (BlockBrowser extension)";
+      return false;
+    }
+  }
+
   // - Shared modules are just resources used by other extensions, and are not
   //   user-controlled.
   if (SharedModuleInfo::IsSharedModule(extension)) {
