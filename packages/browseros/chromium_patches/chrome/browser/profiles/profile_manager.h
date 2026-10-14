diff --git a/chrome/browser/profiles/profile_manager.h b/chrome/browser/profiles/profile_manager.h
index 5f49c6e87f2ea7ae5bf28b94edc4542d524ea1c4..828d6ea7105ae481ad582a7e1ae359f39534f24e 100644
--- a/chrome/browser/profiles/profile_manager.h
+++ b/chrome/browser/profiles/profile_manager.h
@@ -220,6 +220,15 @@ class ProfileManager : public Profile::Delegate {
                          bool incognito,
                          ProfileLoadedCallback callback);
 
+  // BrowserOS automation's existing-only load. Unlike LoadProfileByPath, a
+  // load started here never creates the profile directory or default prefs,
+  // even if the directory disappears during asynchronous initialization. The
+  // callback's bool is true only when this call started initialization;
+  // joining a load already in flight, or an already loaded profile, is false.
+  void LoadExistingProfileByPath(
+      const base::FilePath& profile_path,
+      base::OnceCallback<void(Profile*, bool)> callback);
+
   // Creates or loads the profile located at |profile_path|.
   // Should be called on the UI thread.
   // Params:
