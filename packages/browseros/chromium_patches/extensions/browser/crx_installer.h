diff --git a/extensions/browser/crx_installer.h b/extensions/browser/crx_installer.h
index b12d3bdac536ee86554c72ca6d9e3def25848616..e77160c9fd4914213c3acfeffdf81ad7b27e32b0 100644
--- a/extensions/browser/crx_installer.h
+++ b/extensions/browser/crx_installer.h
@@ -21,6 +21,7 @@
 #include "components/sync/model/string_ordinal.h"
 #include "extensions/browser/extension_install_prompt_client.h"
 #include "extensions/browser/extension_system.h"
+#include "extensions/browser/external_install_info.h"
 #include "extensions/browser/install_flag.h"
 #include "extensions/browser/manifest_check_level.h"
 #include "extensions/browser/preload_check.h"
@@ -249,6 +250,12 @@ class CrxInstaller : public SandboxedUnpackerClient {
   void set_install_immediately(bool val) {
     set_install_flag(kInstallFlagInstallImmediately, val);
   }
+
+  // Set before installation starts: the unpacker owns a sequenced task runner
+  // whose priority is fixed when it is first created. Unlike install_immediately
+  // (which bypasses update delay), this only controls file-work scheduling.
+  void set_external_install_priority(ExternalInstallPriority priority);
+
   void set_do_not_sync(bool val) {
     set_install_flag(kInstallFlagDoNotSync, val);
   }
@@ -546,6 +553,9 @@ class CrxInstaller : public SandboxedUnpackerClient {
   // Sequenced task runner where most file I/O operations will be performed.
   scoped_refptr<base::SequencedTaskRunner> shared_file_task_runner_;
 
+  ExternalInstallPriority external_install_priority_ =
+      ExternalInstallPriority::kDefault;
+
   // Sequenced task runner where the SandboxedUnpacker will run. Because the
   // unpacker uses its own temp dir, it won't hit race conditions, and can use a
   // separate task runner per instance (for better performance).
