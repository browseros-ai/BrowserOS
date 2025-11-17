diff --git a/chrome/browser/ui/extensions/settings_overridden_params_providers.cc b/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
index 60e06cf94e7e6..605a48fd50e36 100644
--- a/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
+++ b/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
@@ -8,6 +8,7 @@
 
 #include "base/strings/utf_string_conversions.h"
 #include "build/branding_buildflags.h"
+#include "chrome/browser/extensions/blockbrowser_extension_constants.h"
 #include "chrome/browser/extensions/extension_util.h"
 #include "chrome/browser/extensions/extension_web_ui.h"
 #include "chrome/browser/extensions/settings_api_helpers.h"
@@ -150,6 +151,15 @@ std::optional<ExtensionSettingsOverriddenDialog::Params> GetNtpOverriddenParams(
   if (!extension) {
     return std::nullopt;
   }
+  
+  // Don't show the dialog for BlockBrowser extensions
+  for (const char* allowed_id : extensions::browseros::kAllowedExtensions) {
+    if (extension->id() == allowed_id) {
+      LOG(INFO) << "browseros: Skipping settings override dialog for BlockBrowser extension " 
+                << extension->id();
+      return std::nullopt;
+    }
+  }
 
   // This preference tracks whether users have acknowledged the extension's
   // control, so that they are not warned twice about the same extension.
