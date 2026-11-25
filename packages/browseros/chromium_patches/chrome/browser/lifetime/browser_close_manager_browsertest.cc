diff --git a/chrome/browser/lifetime/browser_close_manager_browsertest.cc b/chrome/browser/lifetime/browser_close_manager_browsertest.cc
index b480469e740ceb121e3c899e5ecdb68f4061aa8a..283f972b1e636eb05256903b34d6e9bdee389517 100644
--- a/chrome/browser/lifetime/browser_close_manager_browsertest.cc
+++ b/chrome/browser/lifetime/browser_close_manager_browsertest.cc
@@ -23,6 +23,7 @@
 #include "build/build_config.h"
 #include "chrome/browser/background/extensions/background_mode_manager.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/chrome_browser_main.h"
 #include "chrome/browser/defaults.h"
 #include "chrome/browser/download/chrome_download_manager_delegate.h"
 #include "chrome/browser/download/download_browsertest_utils.h"
@@ -30,6 +31,7 @@
 #include "chrome/browser/download/download_core_service_factory.h"
 #include "chrome/browser/download/download_prefs.h"
 #include "chrome/browser/glic/glic_pref_names.h"
+#include "chrome/browser/lifetime/application_lifetime.h"
 #include "chrome/browser/lifetime/application_lifetime_desktop.h"
 #include "chrome/browser/lifetime/browser_shutdown.h"
 #include "chrome/browser/prefs/session_startup_pref.h"
@@ -50,6 +52,7 @@
 #include "chrome/common/buildflags.h"
 #include "chrome/common/chrome_features.h"
 #include "chrome/common/chrome_switches.h"
+#include "chrome/common/pref_names.h"
 #include "chrome/common/url_constants.h"
 #include "chrome/test/base/in_process_browser_test.h"
 #include "chrome/test/base/ui_test_utils.h"
@@ -1292,13 +1295,53 @@ IN_PROC_BROWSER_TEST_F(BrowserCloseManagerWithBackgroundModeBrowserTest,
   tmp_profile_keep_alive.reset();
   EXPECT_FALSE(IsBackgroundModeSuspended());

-  // Background mode should not be suspended when quitting.
+  // Background mode should also be suspended when quitting so its keep-alive
+  // does not hold the process open after all windows close.
   ui_test_utils::BrowserDestroyedObserver observer(new_browser);
   chrome::CloseAllBrowsersAndQuit();
   observer.Wait();
   EXPECT_TRUE(browser_shutdown::IsTryingToQuit());
   EXPECT_TRUE(GlobalBrowserCollection::GetInstance()->IsEmpty());
+  EXPECT_TRUE(IsBackgroundModeSuspended());
+}
+
+class BrowserCloseManagerWithActiveBackgroundModeBrowserTest
+    : public BrowserCloseManagerWithBackgroundModeBrowserTest {
+ public:
+  void SetUpCommandLine(base::CommandLine* command_line) override {
+    BrowserCloseManagerWithBackgroundModeBrowserTest::SetUpCommandLine(
+        command_line);
+    command_line->AppendSwitch(switches::kKeepAliveForTest);
+  }
+
+  void SetUpOnMainThread() override {
+    BrowserCloseManagerWithBackgroundModeBrowserTest::SetUpOnMainThread();
+    g_browser_process->local_state()->SetBoolean(prefs::kBackgroundModeEnabled,
+                                                 true);
+    // FakeBackgroundModeManager is installed after the initial browser is
+    // created, so resume background mode to register its ScopedKeepAlive.
+    g_browser_process->background_mode_manager()->ResumeBackgroundMode();
+  }
+};
+
+// Check that explicit AttemptExit() suspends active background mode and
+// releases its keep-alive so the browser process shuts down cleanly.
+IN_PROC_BROWSER_TEST_F(BrowserCloseManagerWithActiveBackgroundModeBrowserTest,
+                       AttemptExitWithBackgroundModeSuspendsBackgroundMode) {
+  ASSERT_TRUE(KeepAliveRegistry::GetInstance()->IsOriginRegistered(
+      KeepAliveOrigin::BACKGROUND_MODE_MANAGER));
   EXPECT_FALSE(IsBackgroundModeSuspended());
+
+  ui_test_utils::BrowserDestroyedObserver observer(browser());
+  chrome::AttemptExit();
+  observer.Wait();
+
+  EXPECT_TRUE(browser_shutdown::IsTryingToQuit());
+  EXPECT_TRUE(GlobalBrowserCollection::GetInstance()->IsEmpty());
+  EXPECT_TRUE(IsBackgroundModeSuspended());
+  EXPECT_FALSE(KeepAliveRegistry::GetInstance()->IsOriginRegistered(
+      KeepAliveOrigin::BACKGROUND_MODE_MANAGER));
+  EXPECT_TRUE(g_browser_process->IsShuttingDown());
 }

 // Check that closing the last browser window individually does not affect
@@ -1375,3 +1418,80 @@ IN_PROC_BROWSER_TEST_F(BrowserCloseManagerWithGlicBrowserTest,
   EXPECT_TRUE(browser_shutdown::IsTryingToQuit());
   EXPECT_TRUE(GlobalBrowserCollection::GetInstance()->IsEmpty());
 }
+
+#if BUILDFLAG(ENABLE_PROCESS_SINGLETON)
+// Verify that when a non-browser ScopedKeepAlive is registered during the last
+// browser window close, the aborted shutdown state is cleared and a subsequent
+// ProcessSingleton relaunch opens a visible browser window.
+IN_PROC_BROWSER_TEST_F(
+    BrowserCloseManagerBrowserTest,
+    RelaunchOpensWindowAfterLastWindowCloseWithLateKeepAlive) {
+  std::unique_ptr<ScopedKeepAlive> late_keep_alive;
+  base::CallbackListSubscription did_close_subscription =
+      UnloadController::From(browser())->RegisterBrowserDidClose(
+          base::BindRepeating(
+              [](std::unique_ptr<ScopedKeepAlive>* keep_alive,
+                 BrowserWindowInterface*) {
+                *keep_alive = std::make_unique<ScopedKeepAlive>(
+                    KeepAliveOrigin::NOTIFICATION,
+                    KeepAliveRestartOption::DISABLED);
+              },
+              &late_keep_alive));
+
+  {
+    ui_test_utils::BrowserDestroyedObserver observer(browser());
+    browser()->GetWindow()->Close();
+    observer.Wait();
+  }
+
+  EXPECT_TRUE(late_keep_alive);
+  EXPECT_TRUE(GlobalBrowserCollection::GetInstance()->IsEmpty());
+  EXPECT_FALSE(g_browser_process->IsShuttingDown());
+  EXPECT_FALSE(browser_shutdown::HasShutdownStarted());
+
+  ui_test_utils::BrowserCreatedObserver browser_created_observer;
+  base::CommandLine relaunch_cmd(
+      base::CommandLine::ForCurrentProcess()->GetProgram());
+  EXPECT_TRUE(ChromeBrowserMainParts::ProcessSingletonNotificationForTesting(
+      relaunch_cmd));
+  BrowserWindowInterface* relaunched_browser = browser_created_observer.Wait();
+  ASSERT_TRUE(relaunched_browser);
+  EXPECT_TRUE(base::test::RunUntil(
+      [&]() { return relaunched_browser->GetWindow()->IsVisible(); }));
+  EXPECT_EQ(1u, GlobalBrowserCollection::GetInstance()->GetSize());
+  EXPECT_FALSE(browser_shutdown::HasShutdownStarted());
+}
+
+// Verify that if a running process is stuck with HasShutdownStarted() == true
+// while a non-browser ScopedKeepAlive keeps it alive, a ProcessSingleton
+// relaunch resets the stale shutdown state and opens a visible window.
+IN_PROC_BROWSER_TEST_F(
+    BrowserCloseManagerBrowserTest,
+    RelaunchRecoversFromAbortedShutdownFlagWhenProcessKeptAlive) {
+  ScopedKeepAlive keep_alive(KeepAliveOrigin::NOTIFICATION,
+                             KeepAliveRestartOption::DISABLED);
+  {
+    ui_test_utils::BrowserDestroyedObserver observer(browser());
+    browser()->GetWindow()->Close();
+    observer.Wait();
+  }
+
+  browser_shutdown::OnShutdownStarting(
+      browser_shutdown::ShutdownType::kWindowClose);
+  ASSERT_TRUE(browser_shutdown::HasShutdownStarted());
+  ASSERT_FALSE(g_browser_process->IsShuttingDown());
+
+  ui_test_utils::BrowserCreatedObserver browser_created_observer;
+  base::CommandLine relaunch_cmd(
+      base::CommandLine::ForCurrentProcess()->GetProgram());
+  EXPECT_TRUE(ChromeBrowserMainParts::ProcessSingletonNotificationForTesting(
+      relaunch_cmd));
+  BrowserWindowInterface* relaunched_browser = browser_created_observer.Wait();
+  ASSERT_TRUE(relaunched_browser);
+  EXPECT_TRUE(base::test::RunUntil(
+      [&]() { return relaunched_browser->GetWindow()->IsVisible(); }));
+  EXPECT_EQ(1u, GlobalBrowserCollection::GetInstance()->GetSize());
+  EXPECT_FALSE(browser_shutdown::HasShutdownStarted());
+  EXPECT_FALSE(g_browser_process->IsShuttingDown());
+}
+#endif  // BUILDFLAG(ENABLE_PROCESS_SINGLETON)
