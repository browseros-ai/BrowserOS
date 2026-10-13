diff --git a/chrome/browser/profiles/profile_impl.h b/chrome/browser/profiles/profile_impl.h
index 5fd22e22cbfe535d4100b2a40ee0e6b0b57b0526..2a5d80f3c360edb4cf1bef0fd0e206b4f20b490a 100644
--- a/chrome/browser/profiles/profile_impl.h
+++ b/chrome/browser/profiles/profile_impl.h
@@ -191,7 +191,8 @@ class ProfileImpl : public Profile {
               Delegate* delegate,
               CreateMode create_mode,
               base::Time path_creation_time,
-              scoped_refptr<base::SequencedTaskRunner> io_task_runner);
+              scoped_refptr<base::SequencedTaskRunner> io_task_runner,
+              bool create_if_missing);
 
 #if BUILDFLAG(IS_ANDROID)
   // Takes the ownership of the pre-created PrefService and other objects if
@@ -237,6 +238,10 @@ class ProfileImpl : public Profile {
 
   base::FilePath path_;
 
+  // Existing-only automation loads must retain their no-creation policy across
+  // the prefs I/O handoff, before services and cache initialization can run.
+  const bool create_if_missing_;
+
   base::Time path_creation_time_;
 
   // Task runner used for file access in the profile path.
