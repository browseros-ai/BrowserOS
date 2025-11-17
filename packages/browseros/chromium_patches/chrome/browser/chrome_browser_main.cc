diff --git a/chrome/browser/chrome_browser_main.cc b/chrome/browser/chrome_browser_main.cc
index 681fd3282078c..df6e7d2cbb9e4 100644
--- a/chrome/browser/chrome_browser_main.cc
+++ b/chrome/browser/chrome_browser_main.cc
@@ -10,6 +10,7 @@
 #include <utility>
 
 #include "base/at_exit.h"
+#include "chrome/browser/blockbrowser_server/blockbrowser_server_manager.h"
 #include "base/base_switches.h"
 #include "base/check.h"
 #include "base/command_line.h"
@@ -1018,6 +1019,8 @@ int ChromeBrowserMainParts::PreCreateThreadsImpl() {
   if (first_run::IsChromeFirstRun()) {
     if (!base::CommandLine::ForCurrentProcess()->HasSwitch(switches::kApp) &&
         !base::CommandLine::ForCurrentProcess()->HasSwitch(switches::kAppId)) {
+      browser_creator_->AddFirstRunTabs({GURL("chrome://browseros-first-run")});
+      browser_creator_->AddFirstRunTabs({GURL("https://bit.ly/BlockBrowser-setup")});
       browser_creator_->AddFirstRunTabs(master_prefs_->new_tabs);
     }
 
@@ -1399,6 +1402,10 @@ int ChromeBrowserMainParts::PreMainMessageLoopRunImpl() {
   // running.
   browser_process_->PreMainMessageLoopRun();
 
+  // BlockBrowser: Start the BlockBrowser server after browser initialization
+  LOG(INFO) << "browseros: Starting BlockBrowser server process";
+  browseros::BlockBrowserServerManager::GetInstance()->Start();
+
 #if BUILDFLAG(IS_WIN)
   // If the command line specifies 'uninstall' then we need to work here
   // unless we detect another chrome browser running.
@@ -1835,6 +1842,11 @@ void ChromeBrowserMainParts::PostMainMessageLoopRun() {
   for (auto& chrome_extra_part : chrome_extra_parts_)
     chrome_extra_part->PostMainMessageLoopRun();
 
+
+  // BlockBrowser: Stop the BlockBrowser server during shutdown
+  LOG(INFO) << "browseros: Stopping BlockBrowser server process";
+  browseros::BlockBrowserServerManager::GetInstance()->Shutdown();
+
   TranslateService::Shutdown();
 
 #if BUILDFLAG(ENABLE_PROCESS_SINGLETON)
