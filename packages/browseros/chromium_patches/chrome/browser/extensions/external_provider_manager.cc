diff --git a/chrome/browser/extensions/external_provider_manager.cc b/chrome/browser/extensions/external_provider_manager.cc
index 80ac525552c03f173ae13812bce4dc57222dec53..32c79687cabc0bb685a99128549a29a6355d1d10 100644
--- a/chrome/browser/extensions/external_provider_manager.cc
+++ b/chrome/browser/extensions/external_provider_manager.cc
@@ -4,6 +4,7 @@
 
 #include "chrome/browser/extensions/external_provider_manager.h"
 
+#include <algorithm>
 #include <cstddef>
 
 #include "base/check.h"
@@ -14,6 +15,7 @@
 #include "base/trace_event/trace_event.h"
 #include "base/version.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/corrupted_extension_reinstaller.h"
 #include "chrome/browser/extensions/extension_error_controller.h"
 #include "chrome/browser/extensions/external_install_manager.h"
@@ -291,6 +293,7 @@ bool ExternalProviderManager::OnExternalExtensionFileFound(
   installer->set_expected_version(info.version,
                                   true /* fail_install_if_unexpected */);
   installer->set_install_immediately(info.install_immediately);
+  installer->set_external_install_priority(info.install_priority);
   installer->set_creation_flags(info.creation_flags);
 
   CRXFileInfo file_info(
@@ -477,8 +480,21 @@ void ExternalProviderManager::OnExternalProviderUpdateComplete(
   Profile* profile = Profile::FromBrowserContext(context_);
   ExtensionUpdater* updater = ExtensionUpdater::Get(profile);
   if (!update_url_extensions.empty() && updater->enabled()) {
-    // Empty params will cause pending extensions to be updated.
-    updater->CheckNow(ExtensionUpdater::CheckParams());
+    ExtensionUpdater::CheckParams params;
+    if (std::ranges::all_of(update_url_extensions, [](const auto& extension) {
+          return browseros::IsActiveBrowserOSExtension(extension.extension_id);
+        })) {
+      // Product pages such as the cockpit can stay open indefinitely, so an
+      // idle-only update can remain downloaded without ever activating. Reuse
+      // this provider's check and force activation only for its active product
+      // IDs; empty/default params would include unrelated installed extensions.
+      for (const auto& extension : update_url_extensions) {
+        params.ids.push_back(extension.extension_id);
+      }
+      params.install_immediately = true;
+      params.fetch_priority = DownloadFetchPriority::kForeground;
+    }
+    updater->CheckNow(std::move(params));
   }
 
   error_controller_->ShowErrorIfNeeded();
