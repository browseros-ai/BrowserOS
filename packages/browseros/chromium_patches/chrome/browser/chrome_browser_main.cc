diff --git a/chrome/browser/chrome_browser_main.cc b/chrome/browser/chrome_browser_main.cc
index a8ebab6e15ac1..725dce9fa19de 100644
--- a/chrome/browser/chrome_browser_main.cc
+++ b/chrome/browser/chrome_browser_main.cc
@@ -10,6 +10,7 @@
 #include <utility>
 
 #include "base/at_exit.h"
+#include "chrome/browser/browseros/server/browseros_server_manager.h"
 #include "base/base_switches.h"
 #include "base/check.h"
 #include "base/command_line.h"
@@ -1280,6 +1281,43 @@ int ChromeBrowserMainParts::PreCreateThreadsImpl() {
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
@@ -1887,6 +1925,12 @@ int ChromeBrowserMainParts::PreMainMessageLoopRunImpl() {
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
@@ -2175,6 +2219,11 @@ void ChromeBrowserMainParts::PostMainMessageLoopRun() {
     chrome_extra_part->PostMainMessageLoopRun();
   }
 
+
+  // BrowserOS: Stop the BrowserOS server during shutdown
+  LOG(INFO) << "browseros: Stopping BrowserOS server process";
+  browseros::BrowserOSServerManager::GetInstance()->Shutdown();
+
   TranslateService::Shutdown();
 
 #if BUILDFLAG(ENABLE_PROCESS_SINGLETON)
