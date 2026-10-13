diff --git a/chrome/browser/profiles/profile.h b/chrome/browser/profiles/profile.h
index c3f5e8f8e4bbeafef1741b7c8139737177fdb905..94bf2c530825404b9d41b67ebfadca4d1c3d8885 100644
--- a/chrome/browser/profiles/profile.h
+++ b/chrome/browser/profiles/profile.h
@@ -189,11 +189,15 @@ class Profile : public content::BrowserContext {
 
   // Create a new profile given a path. If `create_mode` is kAsynchronous then
   // the profile is initialized asynchronously.
-  // Can return null if `create_mode` is kSynchronous and the creation of
-  // the profile directory fails.
+  // With `create_if_missing=false`, loading never creates the profile directory
+  // or initializes a new profile. This policy also survives asynchronous prefs
+  // loading; a caller-side existence check would leave a recreation race.
+  // Can return null if the directory cannot be created or an existing-only
+  // load cannot find its directory.
   static std::unique_ptr<Profile> CreateProfile(const base::FilePath& path,
                                                 Delegate* delegate,
-                                                CreateMode create_mode);
+                                                CreateMode create_mode,
+                                                bool create_if_missing = true);
 
   // Returns the profile corresponding to the given browser context.
   static Profile* FromBrowserContext(content::BrowserContext* browser_context);
