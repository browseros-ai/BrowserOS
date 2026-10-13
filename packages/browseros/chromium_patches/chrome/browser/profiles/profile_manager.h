diff --git a/chrome/browser/profiles/profile_manager.h b/chrome/browser/profiles/profile_manager.h
index 5f49c6e87f2ea7ae5bf28b94edc4542d524ea1c4..73b877f598d4f7bbcb6baadd1b56a246700dbcfd 100644
--- a/chrome/browser/profiles/profile_manager.h
+++ b/chrome/browser/profiles/profile_manager.h
@@ -220,6 +220,13 @@ class ProfileManager : public Profile::Delegate {
                          bool incognito,
                          ProfileLoadedCallback callback);
 
+  // Automation's existing-only path carries the no-creation policy through
+  // deferred loads and ProfileImpl's asynchronous initialization. The callback
+  // also distinguishes starting initialization from joining an existing load.
+  void LoadExistingProfileByPath(
+      const base::FilePath& profile_path,
+      base::OnceCallback<void(Profile*, bool)> callback);
+
   // Creates or loads the profile located at |profile_path|.
   // Should be called on the UI thread.
   // Params:
@@ -233,7 +240,8 @@ class ProfileManager : public Profile::Delegate {
   void CreateProfileAsync(
       const base::FilePath& profile_path,
       base::OnceCallback<void(Profile*)> initialized_callback,
-      base::OnceCallback<void(Profile*)> created_callback = {});
+      base::OnceCallback<void(Profile*)> created_callback = {},
+      bool create_if_missing = true);
 
   // Returns true if the profile pointer is known to point to an existing
   // profile.
