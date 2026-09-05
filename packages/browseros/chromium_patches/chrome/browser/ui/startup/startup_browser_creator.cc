diff --git a/chrome/browser/ui/startup/startup_browser_creator.cc b/chrome/browser/ui/startup/startup_browser_creator.cc
index bed74e910df0667b636c5b1e404df913febbc019..1ede5472b8f5666bee2fbeca32eb47ea38f5e85a 100644
--- a/chrome/browser/ui/startup/startup_browser_creator.cc
+++ b/chrome/browser/ui/startup/startup_browser_creator.cc
@@ -41,6 +41,9 @@
 #include "chrome/browser/apps/platform_apps/platform_app_launch.h"
 #include "chrome/browser/browser_features.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding_prefs.h"
 #include "chrome/browser/extensions/startup_helper.h"
 #include "chrome/browser/first_run/first_run.h"
 #include "chrome/browser/lifetime/browser_shutdown.h"
@@ -476,6 +479,69 @@ void OpenNewWindowForFirstRun(const base::CommandLine& command_line,
 }
 #endif  // BUILDFLAG(ENABLE_DICE_SUPPORT)
 
+#if !BUILDFLAG(IS_CHROMEOS)
+// Exit callback for the BrowserOS onboarding first-run flow. Opens a browser
+// window whether onboarding completed or was dismissed (close/cancel/crash),
+// so the user is never left without a window.
+void OpenNewWindowForBrowserOSOnboarding(
+    const base::CommandLine& command_line,
+    Profile* profile,
+    const base::FilePath& cur_dir,
+    const std::vector<GURL>& first_run_urls,
+    chrome::startup::IsProcessStartup process_startup,
+    chrome::startup::IsFirstRun is_first_run,
+    ProfilePicker::FirstRunExitStatus status) {
+  // kAbortTask: a newer first-run attempt took over the picker and owns the
+  // launch. kAbandonedFlow: the user reached a browser window some other way
+  // or quit the app from the onboarding window.
+  if (status == ProfilePicker::FirstRunExitStatus::kAbortTask ||
+      status == ProfilePicker::FirstRunExitStatus::kAbandonedFlow) {
+    return;
+  }
+
+  // On Mac, Cmd+Q closes the onboarding window and reports kQuitAtEnd here;
+  // opening a window would fight the in-flight shutdown.
+  if (browser_shutdown::IsTryingToQuit() ||
+      browser_shutdown::HasShutdownStarted()) {
+    return;
+  }
+
+  if (status == ProfilePicker::FirstRunExitStatus::kCompleted) {
+    ProfilePicker::SetOpenCommandLineUrlsInNextProfileOpened(true);
+    // kCompleted is posted only after the target profile's primary extension
+    // enters ExtensionRegistry::ready_extensions(). Do not move this URL into
+    // the picker launch: supplying provider prefs alone cannot authorize it.
+    // BrowserOS onboarding hands off to the agent extension so the user
+    // can finish setting up a provider or coding agent; a chrome:// page
+    // cannot navigate there itself. The agent extension only exists on
+    // BrowserOS builds. BrowserClaw instead hands off to its MCP page.
+    // The BrowserOS target is the first-run setup step, not the settings
+    // page: it drops the user straight into the new tab page once they
+    // connect something.
+    std::vector<GURL> tabs = first_run_urls;
+    if (browseros::IsBrowserOSProduct()) {
+      tabs.emplace_back(std::string("chrome-extension://") +
+                        browseros::kAgentExtensionId +
+                        "/app.html#/onboarding/ai");
+    } else if (browseros::IsBrowserClawProduct()) {
+      // The native callback now owns the final MCP CTA as well; UI resources
+      // must not navigate the picker before its primary extension is ready.
+      tabs.emplace_back("chrome://newtab/#/mcp");
+    }
+    ProfilePicker::SetFirstRunTabsInNextProfileOpened(tabs);
+    return;
+  }
+
+  // Must precede LaunchBrowser(): it re-checks ShouldShow() and would
+  // re-open the onboarding picker mid-teardown.
+  browseros::onboarding::MarkCompleted(profile);
+
+  StartupBrowserCreator browser_creator;
+  browser_creator.LaunchBrowser(command_line, profile, cur_dir, process_startup,
+                                is_first_run, /*restore_tabbed_browser=*/true);
+}
+#endif  // !BUILDFLAG(IS_CHROMEOS)
+
 #if BUILDFLAG(IS_CHROMEOS)
 // Returns the app id of the kiosk app associated with the current user session.
 // Returns nullopt for non-kiosk user sessions and for ARCVM kiosk sessions,
@@ -714,6 +780,26 @@ void StartupBrowserCreator::LaunchBrowser(
       command_line, {profile, StartupProfileMode::kBrowserWindow});
 
   if (!IsSilentLaunchEnabled(command_line, profile)) {
+#if !BUILDFLAG(IS_CHROMEOS)
+    if (browseros::IsBrowserOSProduct()) {
+      browseros::onboarding::NeutralizeUpstreamFirstRun();
+    }
+
+    if (!command_line.HasSwitch(switches::kNoFirstRun) &&
+        browseros::onboarding::ShouldShow(profile)) {
+      // BrowserOS onboarding is now the first-run experience. Stand down
+      // Chromium's DICE first-run so it does not re-intercept the browser
+      // launch when onboarding completes (which would deadlock with no window).
+      browseros::onboarding::NeutralizeUpstreamFirstRun();
+      ProfilePicker::Show(ProfilePicker::Params::ForFirstRun(
+          profile->GetPath(),
+          base::BindOnce(&OpenNewWindowForBrowserOSOnboarding, command_line,
+                         profile, cur_dir, first_run_tabs_, process_startup,
+                         is_first_run)));
+      return;
+    }
+#endif  // !BUILDFLAG(IS_CHROMEOS)
+
 #if BUILDFLAG(ENABLE_DICE_SUPPORT)
     auto* fre_service = FirstRunServiceFactory::GetForBrowserContext(profile);
     if (fre_service && fre_service->ShouldOpenFirstRun()) {
