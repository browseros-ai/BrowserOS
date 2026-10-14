diff --git a/chrome/browser/profiles/profile.h b/chrome/browser/profiles/profile.h
index c3f5e8f8e4bbeafef1741b7c8139737177fdb905..fa3f99a2fd8f90d1fb8992ce7f32bf83891c4a68 100644
--- a/chrome/browser/profiles/profile.h
+++ b/chrome/browser/profiles/profile.h
@@ -191,9 +191,14 @@ class Profile : public content::BrowserContext {
   // the profile is initialized asynchronously.
   // Can return null if `create_mode` is kSynchronous and the creation of
   // the profile directory fails.
+  // BrowserOS: `create_if_missing=false` is an existing-only load. In either
+  // mode it never creates the profile directory and returns null if it is
+  // missing; ProfileImpl re-checks the directory and prefs once they load, so
+  // a directory removed mid-load is not recreated either.
   static std::unique_ptr<Profile> CreateProfile(const base::FilePath& path,
                                                 Delegate* delegate,
-                                                CreateMode create_mode);
+                                                CreateMode create_mode,
+                                                bool create_if_missing = true);
 
   // Returns the profile corresponding to the given browser context.
   static Profile* FromBrowserContext(content::BrowserContext* browser_context);
