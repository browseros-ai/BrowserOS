diff --git a/chrome/browser/profiles/profile_impl.cc b/chrome/browser/profiles/profile_impl.cc
index 81e83a88483367ffb93434cd6223a080ee42593d..9b9e0201dcec49613a651f765600e828bb6247c2 100644
--- a/chrome/browser/profiles/profile_impl.cc
+++ b/chrome/browser/profiles/profile_impl.cc
@@ -361,7 +361,8 @@ GetTestingCloudPolicyManagerFactory() {
 // static
 std::unique_ptr<Profile> Profile::CreateProfile(const base::FilePath& path,
                                                 Delegate* delegate,
-                                                CreateMode create_mode) {
+                                                CreateMode create_mode,
+                                                bool create_if_missing) {
   TRACE_EVENT1("browser,startup", "Profile::CreateProfile", "profile_path",
                path.AsUTF8Unsafe());
 
@@ -372,9 +373,24 @@ std::unique_ptr<Profile> Profile::CreateProfile(const base::FilePath& path,
       base::ThreadPool::CreateSequencedTaskRunner(
           {base::TaskShutdownBehavior::BLOCK_SHUTDOWN, base::MayBlock()});
   base::Time creation_time = base::Time::Now();
+  if (!create_if_missing) {
+    // BrowserOS existing-only load: enforce it at the directory-creation seam,
+    // not merely in the CDP handler, because deferred ProfileManager loads
+    // reach this much later. The directory must exist now, and neither case
+    // below may create it if it disappears after this check.
+    ScopedAllowBlockingForProfile allow_io;
+    if (!base::DirectoryExists(path)) {
+      return nullptr;
+    }
+    creation_time = GetCreationTimeForPath(path);
+  }
   switch (create_mode) {
     case CreateMode::kAsynchronous:
       DCHECK(delegate);
+      if (!create_if_missing) {
+        // Skip CreateProfileDirectory, which recreates a missing directory.
+        break;
+      }
       creation_time = CreateProfileDirectory(io_task_runner.get(), path, true);
       break;
     case CreateMode::kSynchronous:
@@ -384,7 +400,7 @@ std::unique_ptr<Profile> Profile::CreateProfile(const base::FilePath& path,
         // TODO(rogerta): http://crbug.com/40293891 - Bad things happen if we
         // can't write to the profile directory.  We should eventually be able
         // to run in this situation.
-        if (!base::CreateDirectory(path)) {
+        if (!create_if_missing || !base::CreateDirectory(path)) {
           return nullptr;
         }
 
@@ -394,7 +410,8 @@ std::unique_ptr<Profile> Profile::CreateProfile(const base::FilePath& path,
   }
 
   std::unique_ptr<Profile> profile = base::WrapUnique(new ProfileImpl(
-      path, delegate, create_mode, creation_time, io_task_runner));
+      path, delegate, create_mode, creation_time, io_task_runner,
+      create_if_missing));
   return profile;
 }
 
@@ -476,9 +493,11 @@ ProfileImpl::ProfileImpl(
     Delegate* delegate,
     CreateMode create_mode,
     base::Time path_creation_time,
-    scoped_refptr<base::SequencedTaskRunner> io_task_runner)
+    scoped_refptr<base::SequencedTaskRunner> io_task_runner,
+    bool create_if_missing)
     : Profile(nullptr),
       path_(path),
+      create_if_missing_(create_if_missing),
       path_creation_time_(path_creation_time),
       io_task_runner_(std::move(io_task_runner)),
       start_time_(base::Time::Now()),
@@ -791,7 +810,13 @@ void ProfileImpl::DoFinalInit(CreateMode create_mode) {
   // to PathService.
   chrome::GetUserCacheDirectory(path_, &base_cache_path);
   // Always create the cache directory asynchronously.
-  CreateProfileDirectory(io_task_runner_.get(), base_cache_path, false);
+  // A cache path inside the profile must not recreate a deleted profile root
+  // during an existing-only load. Independent platform cache roots keep their
+  // normal initialization behavior.
+  if (create_if_missing_ ||
+      (base_cache_path != path_ && !path_.IsParent(base_cache_path))) {
+    CreateProfileDirectory(io_task_runner_.get(), base_cache_path, false);
+  }
 
   // Initialize components that depend on the current value.
   UpdateSupervisedUserIdInStorage();
@@ -1223,6 +1248,18 @@ void ProfileImpl::OnSyncToSigninMigrationMaybeCompleted(
 
 void ProfileImpl::OnPrefsLoaded(CreateMode create_mode, bool success) {
   TRACE_EVENT0("browser", "ProfileImpl::OnPrefsLoaded");
+  if (success && !create_if_missing_) {
+    // The directory can disappear while prefs are read off the UI thread.
+    // Reject before final initialization, which otherwise creates services,
+    // writes default prefs and makes a missing profile look newly created.
+    ScopedAllowBlockingForProfile allow_io;
+    // IsNewProfile also treats a process-wide first run as onboarding a new
+    // profile, even when its prefs already exist. Only the store's actual
+    // initialization result determines whether this load created defaults.
+    success = base::DirectoryExists(path_) &&
+              GetPrefs()->GetInitializationStatus() ==
+                  PrefService::INITIALIZATION_STATUS_SUCCESS;
+  }
   if (!success) {
     if (delegate_)
       delegate_->OnProfileCreationFinished(this, create_mode, false, false);
