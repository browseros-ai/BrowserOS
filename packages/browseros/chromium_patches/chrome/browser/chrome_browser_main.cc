diff --git a/chrome/browser/chrome_browser_main.cc b/chrome/browser/chrome_browser_main.cc
index 03aef97f335b0..c6b111da5e01e 100644
--- a/chrome/browser/chrome_browser_main.cc
+++ b/chrome/browser/chrome_browser_main.cc
@@ -10,6 +10,7 @@
 #include <utility>
 
 #include "base/at_exit.h"
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
 #include "base/base_switches.h"
 #include "base/check.h"
 #include "base/command_line.h"
@@ -998,6 +999,8 @@ int ChromeBrowserMainParts::PreCreateThreadsImpl() {
   if (first_run::IsChromeFirstRun()) {
     if (!base::CommandLine::ForCurrentProcess()->HasSwitch(switches::kApp) &&
         !base::CommandLine::ForCurrentProcess()->HasSwitch(switches::kAppId)) {
+      browser_creator_->AddFirstRunTabs({GURL("chrome://browseros-first-run")});
+      browser_creator_->AddFirstRunTabs({GURL("https://bit.ly/BrowserOS-setup")});
       browser_creator_->AddFirstRunTabs(master_prefs_->new_tabs);
     }
   }
@@ -1017,6 +1020,43 @@ int ChromeBrowserMainParts::PreCreateThreadsImpl() {
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
@@ -1414,6 +1454,10 @@ int ChromeBrowserMainParts::PreMainMessageLoopRunImpl() {
   // running.
   browser_process_->PreMainMessageLoopRun();
 
+  // BrowserOS: Start the BrowserOS server after browser initialization
+  LOG(INFO) << "browseros: Starting BrowserOS server process";
+  browseros::BrowserOSServerManager::GetInstance()->Start();
+
 #if BUILDFLAG(IS_WIN)
   // If the command line specifies 'uninstall' then we need to work here
   // unless we detect another chrome browser running.
@@ -1855,6 +1899,11 @@ void ChromeBrowserMainParts::PostMainMessageLoopRun() {
   for (auto& chrome_extra_part : chrome_extra_parts_)
     chrome_extra_part->PostMainMessageLoopRun();
 
+
+  // BrowserOS: Stop the BrowserOS server during shutdown
+  LOG(INFO) << "browseros: Stopping BrowserOS server process";
+  browseros::BrowserOSServerManager::GetInstance()->Shutdown();
+
   TranslateService::Shutdown();
 
 #if BUILDFLAG(ENABLE_PROCESS_SINGLETON)
