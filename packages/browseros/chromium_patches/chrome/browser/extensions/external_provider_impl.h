diff --git a/chrome/browser/extensions/external_provider_impl.h b/chrome/browser/extensions/external_provider_impl.h
index 3959d466ab6d3fc30f22f4d4fab51fbf06b4c943..e03613dc8238d616dbd3ee26bb3f4a05365d517d 100644
--- a/chrome/browser/extensions/external_provider_impl.h
+++ b/chrome/browser/extensions/external_provider_impl.h
@@ -15,6 +15,7 @@
 #include "base/memory/scoped_refptr.h"
 #include "base/values.h"
 #include "chrome/browser/extensions/external_loader.h"
+#include "extensions/browser/external_install_info.h"
 #include "extensions/browser/external_provider_interface.h"
 #include "extensions/buildflags/buildflags.h"
 #include "extensions/common/manifest.h"
@@ -101,6 +102,12 @@ class ExternalProviderImpl : public ExternalProviderInterface {
 
   void set_allow_updates(bool allow_updates) { allow_updates_ = allow_updates; }
 
+  // Applies to CRX file announcements only; remote updater work keeps its own
+  // scheduling policy. This does not change provenance or update-delay rules.
+  void set_external_install_priority(ExternalInstallPriority priority) {
+    external_install_priority_ = priority;
+  }
+
   const std::optional<base::DictValue>& prefs_for_test() { return prefs_; }
 
  private:
@@ -159,6 +166,9 @@ class ExternalProviderImpl : public ExternalProviderInterface {
   // Whether the extensions from this provider should be installed immediately.
   bool install_immediately_ = false;
 
+  ExternalInstallPriority external_install_priority_ =
+      ExternalInstallPriority::kDefault;
+
   // Whether the provider should be allowed to update the set of external
   // extensions it provides.
   bool allow_updates_ = false;
