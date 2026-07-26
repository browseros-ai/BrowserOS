diff --git a/chrome/browser/ui/startup/startup_browser_creator.cc b/chrome/browser/ui/startup/startup_browser_creator.cc
index 597bd5bfdcbbfbdcb553639ba24ff01d463ee11e..2bf1f18642288671739c60fca75c392582a78a4d 100644
--- a/chrome/browser/ui/startup/startup_browser_creator.cc
+++ b/chrome/browser/ui/startup/startup_browser_creator.cc
@@ -41,6 +41,8 @@
 #include "chrome/browser/apps/platform_apps/platform_app_launch.h"
 #include "chrome/browser/browser_features.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding_prefs.h"
 #include "chrome/browser/extensions/startup_helper.h"
 #include "chrome/browser/first_run/first_run.h"
 #include "chrome/browser/lifetime/browser_shutdown.h"
@@ -474,6 +476,49 @@ void OpenNewWindowForFirstRun(const base::CommandLine& command_line,
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
+    ProfilePicker::SetFirstRunTabsInNextProfileOpened(first_run_urls);
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
@@ -712,6 +757,26 @@ void StartupBrowserCreator::LaunchBrowser(
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
