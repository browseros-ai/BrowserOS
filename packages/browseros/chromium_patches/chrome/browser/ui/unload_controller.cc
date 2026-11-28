diff --git a/chrome/browser/ui/unload_controller.cc b/chrome/browser/ui/unload_controller.cc
index 201af07cf7cbe30e3032783cee1f6f7f9bfbc432..0b52301c1d8b1985bc52493375a25b6566e7da97 100644
--- a/chrome/browser/ui/unload_controller.cc
+++ b/chrome/browser/ui/unload_controller.cc
@@ -948,35 +948,47 @@ void UnloadController::OnWindowCloseComplete() {
   // deletion.
   browser_did_close_callback_list_.Notify(browser_);

-  // Application should shutdown on last window close if the user is
-  // explicitly trying to quit, or if there is nothing keeping the browser
-  // alive (such as AppController on the Mac, or BackgroundContentsService for
-  // background pages).
-  const bool should_quit_if_last_browser =
-      browser_shutdown::IsTryingToQuit() ||
-      KeepAliveRegistry::GetInstance()->IsKeepingAliveOnlyByBrowserOrigin();
-
-  // Below will not consider browsers for which delete has already been
-  // scheduled.
-  const bool is_last_browser =
-      !GetLastActiveBrowserWindowInterfaceWithAnyProfile();
-
-  if (should_quit_if_last_browser && is_last_browser) {
-    browser_shutdown::OnShutdownStarting(
-        browser_shutdown::ShutdownType::kWindowClose);
-  }
-
   // Once a Browser has successfully closed, client code expects control to
   // return to the run loop before the instance is finally deleted. To
   // maintain existing expectations schedule the delete asynchronously here.
+  // Defer OnShutdownStarting() into the same task so any non-browser
+  // ScopedKeepAlive registered during close callbacks is observed first, and
+  // roll back kWindowClose if a keep-alive still keeps the process running.
   // TODO(crbug.com/413168662): Explore synchronously destroying the browser
   // instead.
   base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
       FROM_HERE,
       base::BindOnce(
           [](base::WeakPtr<BrowserWindowInterface> browser) {
-            if (browser) {
-              BrowserManagerService::SynchronouslyDestroyBrowser(browser.get());
+            if (!browser) {
+              return;
+            }
+            // Application should shutdown on last window close if the user is
+            // explicitly trying to quit, or if there is nothing keeping the
+            // browser alive (such as AppController on the Mac, or
+            // BackgroundContentsService for background pages).
+            const bool should_quit_if_last_browser =
+                browser_shutdown::IsTryingToQuit() ||
+                KeepAliveRegistry::GetInstance()
+                    ->IsKeepingAliveOnlyByBrowserOrigin();
+
+            // Below will not consider browsers for which delete has already
+            // been scheduled.
+            const bool is_last_browser =
+                !GetLastActiveBrowserWindowInterfaceWithAnyProfile();
+
+            if (should_quit_if_last_browser && is_last_browser) {
+              browser_shutdown::OnShutdownStarting(
+                  browser_shutdown::ShutdownType::kWindowClose);
+            }
+
+            BrowserManagerService::SynchronouslyDestroyBrowser(browser.get());
+
+            if (g_browser_process && !g_browser_process->IsShuttingDown() &&
+                !browser_shutdown::IsTryingToQuit() &&
+                browser_shutdown::GetShutdownType() ==
+                    browser_shutdown::ShutdownType::kWindowClose) {
+              browser_shutdown::ResetAbortedShutdown();
             }
           },
           browser_->GetWeakPtr()));
