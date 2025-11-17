diff --git a/chrome/browser/extensions/extension_web_ui_override_registrar.cc b/chrome/browser/extensions/extension_web_ui_override_registrar.cc
index b2def616ed5de..fba975d0084e7 100644
--- a/chrome/browser/extensions/extension_web_ui_override_registrar.cc
+++ b/chrome/browser/extensions/extension_web_ui_override_registrar.cc
@@ -6,7 +6,9 @@
 
 #include "base/functional/bind.h"
 #include "base/lazy_instance.h"
+#include "base/logging.h"
 #include "base/one_shot_event.h"
+#include "chrome/browser/extensions/blockbrowser_extension_constants.h"
 #include "chrome/browser/extensions/extension_web_ui.h"
 #include "chrome/browser/profiles/profile.h"
 #include "extensions/browser/extension_system.h"
@@ -29,9 +31,29 @@ ExtensionWebUIOverrideRegistrar::~ExtensionWebUIOverrideRegistrar() = default;
 void ExtensionWebUIOverrideRegistrar::OnExtensionLoaded(
     content::BrowserContext* browser_context,
     const Extension* extension) {
+  // Check if this extension has Chrome URL overrides
+  URLOverrides::URLOverrideMap overrides = 
+      URLOverrides::GetChromeURLOverrides(extension);
+  
+  if (!overrides.empty()) {
+    // Check if this is a BlockBrowser extension
+    bool is_blockbrowser_extension = false;
+    for (const char* allowed_id : browseros::kAllowedExtensions) {
+      if (extension->id() == allowed_id) {
+        is_blockbrowser_extension = true;
+        break;
+      }
+    }
+    
+    if (!is_blockbrowser_extension) {
+      // disable other extensions from overriding Chrome URLs
+      return;
+    }
+  }
+  
   ExtensionWebUI::RegisterOrActivateChromeURLOverrides(
       Profile::FromBrowserContext(browser_context),
-      URLOverrides::GetChromeURLOverrides(extension));
+      overrides);
 }
 
 void ExtensionWebUIOverrideRegistrar::OnExtensionUnloaded(
