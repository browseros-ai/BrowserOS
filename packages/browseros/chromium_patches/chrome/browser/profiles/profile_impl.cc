diff --git a/chrome/browser/profiles/profile_impl.cc b/chrome/browser/profiles/profile_impl.cc
index 81e83a88483367ffb93434cd6223a080ee42593d..0c26a767b6f7cf20bdda42d990880dea13d812bd 100644
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
 
@@ -372,29 +373,41 @@ std::unique_ptr<Profile> Profile::CreateProfile(const base::FilePath& path,
       base::ThreadPool::CreateSequencedTaskRunner(
           {base::TaskShutdownBehavior::BLOCK_SHUTDOWN, base::MayBlock()});
   base::Time creation_time = base::Time::Now();
-  switch (create_mode) {
-    case CreateMode::kAsynchronous:
-      DCHECK(delegate);
-      creation_time = CreateProfileDirectory(io_task_runner.get(), path, true);
-      break;
-    case CreateMode::kSynchronous:
-      if (base::PathExists(path)) {
-        creation_time = GetCreationTimeForPath(path);
-      } else {
-        // TODO(rogerta): http://crbug.com/40293891 - Bad things happen if we
-        // can't write to the profile directory.  We should eventually be able
-        // to run in this situation.
-        if (!base::CreateDirectory(path)) {
-          return nullptr;
+  if (!create_if_missing) {
+    // Enforce this at the directory-creation seam, not merely in the CDP
+    // handler. Deferred ProfileManager loads reach this code much later.
+    ScopedAllowBlockingForProfile allow_io;
+    if (!base::DirectoryExists(path)) {
+      return nullptr;
+    }
+    creation_time = GetCreationTimeForPath(path);
+  } else {
+    switch (create_mode) {
+      case CreateMode::kAsynchronous:
+        DCHECK(delegate);
+        creation_time =
+            CreateProfileDirectory(io_task_runner.get(), path, true);
+        break;
+      case CreateMode::kSynchronous:
+        if (base::PathExists(path)) {
+          creation_time = GetCreationTimeForPath(path);
+        } else {
+          // TODO(rogerta): http://crbug.com/40293891 - Bad things happen if we
+          // can't write to the profile directory.  We should eventually be able
+          // to run in this situation.
+          if (!base::CreateDirectory(path)) {
+            return nullptr;
+          }
+
+          CreateProfileReadme(path);
         }
-
-        CreateProfileReadme(path);
-      }
-      break;
+        break;
+    }
   }
 
-  std::unique_ptr<Profile> profile = base::WrapUnique(new ProfileImpl(
-      path, delegate, create_mode, creation_time, io_task_runner));
+  std::unique_ptr<Profile> profile = base::WrapUnique(
+      new ProfileImpl(path, delegate, create_mode, creation_time,
+                      io_task_runner, create_if_missing));
   return profile;
 }
 
@@ -476,9 +489,11 @@ ProfileImpl::ProfileImpl(
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
@@ -791,7 +806,13 @@ void ProfileImpl::DoFinalInit(CreateMode create_mode) {
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
@@ -1223,9 +1244,22 @@ void ProfileImpl::OnSyncToSigninMigrationMaybeCompleted(
 
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
-    if (delegate_)
+    if (delegate_) {
       delegate_->OnProfileCreationFinished(this, create_mode, false, false);
+    }
     return;
   }
 
