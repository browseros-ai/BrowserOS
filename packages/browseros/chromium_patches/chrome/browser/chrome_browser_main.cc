diff --git a/chrome/browser/chrome_browser_main.cc b/chrome/browser/chrome_browser_main.cc
index cbf3b6c563090a64c427e1b0ce8cbf7118aa82d2..d7e0df4e0d32f03c726428f932565b98b356424b 100644
--- a/chrome/browser/chrome_browser_main.cc
+++ b/chrome/browser/chrome_browser_main.cc
@@ -34,6 +34,8 @@
 #include "chrome/browser/browser_features.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/browser_process_impl.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics_extra_parts.h"
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
 #include "chrome/browser/chrome_browser_main_extra_parts.h"
 #include "chrome/browser/component_updater/registration.h"
 #include "chrome/browser/enterprise/browser_management/management_service_factory.h"
@@ -843,6 +845,7 @@ std::unique_ptr<content::BrowserMainParts> ChromeBrowserMainParts::Create(
   main_parts->AddParts(std::make_unique<ChromeBrowserMainExtraPartsMemory>());
 
   chrome::AddMetricsExtraParts(main_parts.get());
+  browseros_metrics::AddBrowserOSMetricsExtraParts(main_parts.get());
 
   main_parts->AddParts(
       std::make_unique<
@@ -1359,6 +1362,43 @@ int ChromeBrowserMainParts::PreCreateThreadsImpl() {
   }
 #endif
 
+#if BUILDFLAG(IS_MAC)
+  // Install iCloud Passwords native messaging host manifest.
+  //
+  // Why this runs on every startup (not just first run):
+  // - First-run only would miss existing users upgrading to this version
+  // - The "First Run" sentinel already exists for them, so IsChromeFirstRun()
+  //   returns false and first-run code is skipped entirely
+  // - Running every startup also self-heals if the manifest is deleted
+  // - The PathExists check makes this cheap (~0.1ms) when file already exists
+  {
+    base::FilePath native_messaging_dir;
+    if (base::PathService::Get(chrome::DIR_USER_NATIVE_MESSAGING,
+                               &native_messaging_dir)) {
+      // Ensure directory exists for users who installed before first-run
+      // directory creation was added.
+      if (!base::PathExists(native_messaging_dir))
+        base::CreateDirectory(native_messaging_dir);
+
+      const base::FilePath manifest_path =
+          native_messaging_dir.Append("com.apple.passwordmanager.json");
+      if (!base::PathExists(manifest_path)) {
+        constexpr std::string_view kICloudPasswordsManifest = R"({
+    "name": "com.apple.passwordmanager",
+    "description": "PasswordManagerBrowserExtensionHelper",
+    "path": "/System/Cryptexes/App/System/Library/CoreServices/PasswordManagerBrowserExtensionHelper.app/Contents/MacOS/PasswordManagerBrowserExtensionHelper",
+    "type": "stdio",
+    "allowed_origins": [
+        "chrome-extension://pejdijmoenmkgeppbflobdenhhabjlaj/",
+        "chrome-extension://mfbcdcnpokpoajjciilocoachedjkima/"
+    ]
+})";
+        base::WriteFile(manifest_path, kICloudPasswordsManifest);
+      }
+    }
+  }
+#endif  // BUILDFLAG(IS_MAC)
+
 #if BUILDFLAG(IS_MAC)
 #if defined(ARCH_CPU_X86_64)
   // The use of Rosetta to run the x64 version of Chromium on Arm is neither
@@ -1963,6 +2003,12 @@ int ChromeBrowserMainParts::PreMainMessageLoopRunImpl() {
     g_browser_process->CreateDevToolsAutoOpener();
   }
 
+  // BrowserOS: Start AFTER CreateDevToolsProtocolHandler so that BrowserOS's
+  // CDP handler replaces Chromium's (StartRemoteDebuggingServer is a global
+  // singleton — the last caller wins).
+  LOG(INFO) << "browseros: Starting BrowserOS server process";
+  browseros::BrowserOSServerManager::GetInstance()->Start();
+
   // Needs to be done before PostProfileInit, since the SODA Installer setup is
   // called inside PostProfileInit and depends on it.
   if (!base::CommandLine::ForCurrentProcess()->HasSwitch(
@@ -2267,6 +2313,10 @@ void ChromeBrowserMainParts::PostMainMessageLoopRun() {
     chrome_extra_part->PostMainMessageLoopRun();
   }
 
+  // BrowserOS: Stop the BrowserOS server during shutdown
+  LOG(INFO) << "browseros: Stopping BrowserOS server process";
+  browseros::BrowserOSServerManager::GetInstance()->Shutdown();
+
   TranslateService::Shutdown();
 
 #if BUILDFLAG(ENABLE_PROCESS_SINGLETON)
