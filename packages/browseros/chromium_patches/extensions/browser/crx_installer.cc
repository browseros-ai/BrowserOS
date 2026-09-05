diff --git a/extensions/browser/crx_installer.cc b/extensions/browser/crx_installer.cc
index d093f443863b06d96b091889ddcef71eb192084f..6c519923e6ebd8d8e346166180c51cc23669ea52 100644
--- a/extensions/browser/crx_installer.cc
+++ b/extensions/browser/crx_installer.cc
@@ -1293,9 +1293,20 @@ void CrxInstaller::ConfirmReEnable() {
   }
 }
 
+void CrxInstaller::set_external_install_priority(
+    ExternalInstallPriority priority) {
+  DCHECK_CURRENTLY_ON(BrowserThread::UI);
+  CHECK(!unpacker_task_runner_);
+  external_install_priority_ = priority;
+}
+
 base::SequencedTaskRunner* CrxInstaller::GetUnpackerTaskRunner() {
   if (!unpacker_task_runner_) {
+    // Foreground external files unblock a visible setup surface. Keep the
+    // historical policy for all other installs, including future updater work;
+    // scheduler urgency must not be encoded in persisted creation flags.
     bool low_priority =
+        external_install_priority_ != ExternalInstallPriority::kForeground &&
         (creation_flags_ & Extension::WAS_INSTALLED_BY_DEFAULT) &&
         !(creation_flags_ & Extension::WAS_INSTALLED_BY_OEM);
     unpacker_task_runner_ = GetOneShotFileTaskRunner(
