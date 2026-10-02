diff --git a/chrome/installer/setup/setup_main.cc b/chrome/installer/setup/setup_main.cc
index 8689571a44a1393ac6afde429d7302cf336d1814..33b8bd852a17d909c959afb3034b9b00a6e478fb 100644
--- a/chrome/installer/setup/setup_main.cc
+++ b/chrome/installer/setup/setup_main.cc
@@ -50,7 +50,6 @@
 #include "base/task/single_thread_task_executor.h"
 #include "base/threading/platform_thread.h"
 #include "base/time/time.h"
-#include "base/types/expected_macros.h"
 #include "base/values.h"
 #include "base/version.h"
 #include "base/win/current_module.h"
@@ -72,6 +71,7 @@
 #include "chrome/install_static/install_details.h"
 #include "chrome/install_static/install_util.h"
 #include "chrome/installer/setup/brand_behaviors.h"
+#include "chrome/installer/setup/browseros_install_ui.h"
 #include "chrome/installer/setup/configure_app_container_sandbox.h"
 #include "chrome/installer/setup/downgrade_cleanup.h"
 #include "chrome/installer/setup/install.h"
@@ -1319,12 +1319,55 @@ bool HandleNonInstallCmdLineOptions(installer::ModifyParams& modify_params,
 
 namespace installer {
 
+namespace {
+
+bool ShouldShowBrowserOSInstallUI(const InstallationState& original_state,
+                                  const base::CommandLine& cmd_line,
+                                  const InstallerState& installer_state) {
+  const bool system_install = installer_state.system_install();
+  return installer_state.operation() ==
+             InstallerState::SINGLE_INSTALL_OR_UPDATE &&
+         !system_install && !installer_state.is_msi() &&
+         !cmd_line.HasSwitch(switches::kMsi) &&
+         !cmd_line.HasSwitch(switches::kSilent) &&
+         original_state.GetProductState(system_install) == nullptr;
+}
+
+int GetInstallFailureMessageBase(InstallStatus status) {
+  switch (status) {
+    case TEMP_DIR_FAILED:
+      return IDS_INSTALL_TEMP_DIR_FAILED_BASE;
+    case UNCOMPRESSION_FAILED:
+    case UNPACKING_FAILED:
+      return IDS_INSTALL_UNCOMPRESSION_FAILED_BASE;
+    case INVALID_ARCHIVE:
+      return IDS_INSTALL_INVALID_ARCHIVE_BASE;
+    case HIGHER_VERSION_EXISTS:
+      return IDS_INSTALL_HIGHER_VERSION_BASE;
+    case SAME_VERSION_REPAIR_FAILED:
+      return IDS_SAME_VERSION_REPAIR_FAILED_BASE;
+    case OS_ERROR:
+      return IDS_INSTALL_OS_ERROR_BASE;
+    case INSUFFICIENT_RIGHTS:
+      return IDS_INSTALL_INSUFFICIENT_RIGHTS_BASE;
+    case SETUP_SINGLETON_ACQUISITION_FAILED:
+      return IDS_INSTALL_SINGLETON_ACQUISITION_FAILED_BASE;
+    default:
+      return IDS_INSTALL_FAILED_BASE;
+  }
+}
+
+}  // namespace
+
 InstallStatus InstallProductsHelper(InstallationState& original_state,
                                     const base::FilePath& setup_exe,
                                     const base::CommandLine& cmd_line,
                                     const InitialPreferences& prefs,
                                     InstallerState& installer_state) {
   const bool system_install = installer_state.system_install();
+  BrowserOSInstallUI install_ui(
+      ShouldShowBrowserOSInstallUI(original_state, cmd_line, installer_state));
+  install_ui.Show();
 
   // Create a temp folder where we will unpack Chrome archive. If it fails,
   // then we are doomed, so return immediately and no cleanup is required.
@@ -1334,15 +1377,25 @@ InstallStatus InstallProductsHelper(InstallationState& original_state,
                                            &unpack_path)) {
     installer_state.WriteInstallerResult(
         TEMP_DIR_FAILED, IDS_INSTALL_TEMP_DIR_FAILED_BASE, nullptr);
+    install_ui.CloseAndShowFailureMessage(
+        GetInstallFailureMessageBase(TEMP_DIR_FAILED));
     return TEMP_DIR_FAILED;
   }
 
-  RETURN_IF_ERROR(
-      UnpackChromeArchive(unpack_path, setup_exe, cmd_line, installer_state));
+  // Archive unpacking records the installer result; close the UI thread
+  // before presenting its failure and returning from setup.
+  const auto unpack_result = UnpackChromeArchive(
+      unpack_path, setup_exe, cmd_line, installer_state);
+  if (!unpack_result.has_value()) {
+    install_ui.CloseAndShowFailureMessage(
+        GetInstallFailureMessageBase(unpack_result.error()));
+    return unpack_result.error();
+  }
 
   VLOG(1) << "unpacked to " << unpack_path.value();
 
   InstallStatus install_status = UNKNOWN_STATUS;
+  int failure_msg_base = IDS_INSTALL_FAILED_BASE;
   base::FilePath src_path(unpack_path.Append(kInstallSourceChromeDir));
   std::optional<uint32_t> src_size_kb;
   std::unique_ptr<base::Version> installer_version(
@@ -1350,6 +1403,7 @@ InstallStatus InstallProductsHelper(InstallationState& original_state,
   if (!installer_version.get()) {
     LOG(ERROR) << "Did not find any valid version in installer.";
     install_status = INVALID_ARCHIVE;
+    failure_msg_base = GetInstallFailureMessageBase(install_status);
     installer_state.WriteInstallerResult(
         install_status, IDS_INSTALL_INVALID_ARCHIVE_BASE, nullptr);
   } else {
@@ -1364,6 +1418,7 @@ InstallStatus InstallProductsHelper(InstallationState& original_state,
         int message_id = IDS_INSTALL_HIGHER_VERSION_BASE;
         proceed_with_installation = false;
         install_status = HIGHER_VERSION_EXISTS;
+        failure_msg_base = message_id;
         installer_state.WriteInstallerResult(install_status, message_id,
                                              nullptr);
       }
@@ -1406,6 +1461,8 @@ InstallStatus InstallProductsHelper(InstallationState& original_state,
           install_msg_base = 0;
         }
       }
+      failure_msg_base =
+          install_msg_base != 0 ? install_msg_base : IDS_INSTALL_FAILED_BASE;
 
       installer_state.SetStage(FINISHING);
 
@@ -1422,10 +1479,13 @@ InstallStatus InstallProductsHelper(InstallationState& original_state,
 
       if (install_status == FIRST_INSTALL_SUCCESS) {
         VLOG(1) << "First install successful.";
+        install_ui.Close();
         // We never want to launch Chrome in system level install mode.
         bool do_not_launch_chrome = false;
         prefs.GetBool(initial_preferences::kDoNotLaunchChrome,
                       &do_not_launch_chrome);
+        do_not_launch_chrome =
+            do_not_launch_chrome || cmd_line.HasSwitch(switches::kSilent);
         if (!system_install && !do_not_launch_chrome) {
           LaunchChromeBrowser(installer_state.target_path());
         }
@@ -1480,6 +1540,9 @@ InstallStatus InstallProductsHelper(InstallationState& original_state,
   // temp_path's dtor will take care of deleting or scheduling itself for
   // deletion at reboot when this scope closes.
   VLOG(1) << "Deleting temporary directory " << temp_path.path().value();
+  if (InstallUtil::GetInstallReturnCode(install_status) != 0) {
+    install_ui.CloseAndShowFailureMessage(failure_msg_base);
+  }
 
   return install_status;
 }
