diff --git a/chrome/browser/extensions/updater/extension_updater.h b/chrome/browser/extensions/updater/extension_updater.h
index c5cb7f140455a626910eae1ea4041770dce88d16..b9b8e6ff2c3a71a08d261fc92fa0e4e2743b366b 100644
--- a/chrome/browser/extensions/updater/extension_updater.h
+++ b/chrome/browser/extensions/updater/extension_updater.h
@@ -166,6 +166,13 @@ class ExtensionUpdater : public KeyedService,
   // regularly scheduled check or a pending check from CheckSoon().
   void CheckNow(CheckParams params);
 
+  // Immediately installs pending extensions with the given IDs.
+  // Unlike CheckNow() with specific IDs (which checks installed extensions),
+  // this method specifically targets extensions in the PendingExtensionManager.
+  // Use this when you need to force-install extensions that have been registered
+  // as pending but haven't been downloaded yet.
+  void InstallPendingNow(CheckParams params);
+
   // Returns true iff CheckSoon() has been called but the update check
   // hasn't been performed yet.  This is used mostly by tests; calling
   // code should just call CheckSoon().
