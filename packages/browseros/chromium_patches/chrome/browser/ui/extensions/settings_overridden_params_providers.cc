diff --git a/chrome/browser/ui/extensions/settings_overridden_params_providers.cc b/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
index 59a08a4e0ec57771c631408eebaec98aee6562bd..7b595864cf4ee9787d65a0ad5ce2213e408a1cfc 100644
--- a/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
+++ b/chrome/browser/ui/extensions/settings_overridden_params_providers.cc
@@ -11,6 +11,7 @@
 
 #include "base/barrier_closure.h"
 #include "base/functional/callback_forward.h"
+#include "base/logging.h"
 #include "base/memory/raw_ptr.h"
 #include "base/metrics/histogram_functions.h"
 #include "base/strings/utf_string_conversions.h"
@@ -18,6 +19,7 @@
 #include "base/task/cancelable_task_tracker.h"
 #include "base/task/single_thread_task_runner.h"
 #include "build/branding_buildflags.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_url_overrides.h"
 #include "chrome/browser/extensions/settings_api_helpers.h"
 #include "chrome/browser/image_fetcher/image_fetcher_service_factory.h"
@@ -407,6 +409,13 @@ std::optional<ExtensionSettingsOverriddenDialog::Params> GetNtpOverriddenParams(
     return std::nullopt;
   }
 
+  if (browseros::IsActiveBrowserOSExtension(extension->id())) {
+    LOG(INFO) << "browseros: Skipping settings override dialog for BrowserOS "
+                 "extension "
+              << extension->id();
+    return std::nullopt;
+  }
+
   // This preference tracks whether users have acknowledged the extension's
   // control, so that they are not warned twice about the same extension.
   const char* preference_name = extensions::kNtpOverridingExtensionAcknowledged;
