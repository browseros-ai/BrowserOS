diff --git a/chrome/browser/ui/extensions/settings_overridden_params_providers.cc b/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
index b01073c9f69c9..88a20426a8d1f 100644
--- a/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
+++ b/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
@@ -8,6 +8,7 @@
 
 #include "base/strings/utf_string_conversions.h"
 #include "build/branding_buildflags.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_util.h"
 #include "chrome/browser/extensions/extension_web_ui.h"
 #include "chrome/browser/extensions/settings_api_helpers.h"
@@ -173,6 +174,13 @@ std::optional<ExtensionSettingsOverriddenDialog::Params> GetNtpOverriddenParams(
   if (!extension) {
     return std::nullopt;
   }
+  
+  // Don't show the dialog for BrowserOS extensions
+  if (browseros::IsBrowserOSExtension(extension->id())) {
+    LOG(INFO) << "browseros: Skipping settings override dialog for BrowserOS extension "
+              << extension->id();
+    return std::nullopt;
+  }
 
   // This preference tracks whether users have acknowledged the extension's
   // control, so that they are not warned twice about the same extension.
