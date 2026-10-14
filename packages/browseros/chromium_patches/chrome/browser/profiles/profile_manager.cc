diff --git a/chrome/browser/profiles/profile_manager.cc b/chrome/browser/profiles/profile_manager.cc
index 5856f418248ac92b8e695c1bfe54ad1ab13b1360..271189f5849b8eec6486f6dea6fd03dec74e1a31 100644
--- a/chrome/browser/profiles/profile_manager.cc
+++ b/chrome/browser/profiles/profile_manager.cc
@@ -832,6 +832,64 @@ bool ProfileManager::LoadProfileByPath(const base::FilePath& profile_path,
   return true;
 }
 
+void ProfileManager::LoadExistingProfileByPath(
+    const base::FilePath& profile_path,
+    base::OnceCallback<void(Profile*, bool)> callback) {
+  DCHECK_CURRENTLY_ON(BrowserThread::UI);
+  // Mirror CreateProfileAsync's startup/unloading deferrals before deciding
+  // ownership. Counting a deferred request as the owner could report a new
+  // load when another Chromium caller initializes the profile first. After
+  // this point the stock CreateProfileAsync call below runs in this same task,
+  // so its own deferral branches cannot fire.
+  if (defer_async_loading_) {
+    deferred_asynchronous_loads_.push_back(base::BindOnce(
+        &ProfileManager::LoadExistingProfileByPath, base::Unretained(this),
+        profile_path, std::move(callback)));
+    return;
+  }
+  if (auto pending_it = profiles_pending_destruction_.find(profile_path);
+      pending_it != profiles_pending_destruction_.end()) {
+    pending_it->second.push_back(base::BindOnce(
+        &ProfileManager::LoadExistingProfileByPath, weak_factory_.GetWeakPtr(),
+        profile_path, std::move(callback)));
+    return;
+  }
+  if (!CanCreateProfileAtPath(profile_path) ||
+      !GetProfileAttributesStorage().GetProfileAttributesWithPath(
+          profile_path)) {
+    std::move(callback).Run(nullptr, false);
+    return;
+  }
+  const bool newly_loaded = !profiles_info_.contains(profile_path);
+  if (newly_loaded) {
+    // Start the load here rather than in CreateProfileAsync, whose
+    // CreateProfileAsyncHelper creates a missing directory. Like the stock
+    // path, register the Profile before any callback is enqueued: its prefs
+    // load asynchronously, so OnProfileCreationFinished cannot run before
+    // CreateProfileAsync below appends to init_callbacks.
+    std::unique_ptr<Profile> profile = Profile::CreateProfile(
+        profile_path, this, Profile::CreateMode::kAsynchronous,
+        /*create_if_missing=*/false);
+    if (!profile) {
+      std::move(callback).Run(nullptr, false);
+      return;
+    }
+    RegisterOwnedProfile(std::move(profile));
+  }
+  // Join through the stock path, which finds the registered ProfileInfo and
+  // either runs the callback now (already initialized) or queues it until
+  // OnProfileCreationFinished. Only the call that registered above reports
+  // `newly_loaded`; joining another caller's load reports false.
+  CreateProfileAsync(profile_path,
+                     base::BindOnce(
+                         [](base::OnceCallback<void(Profile*, bool)> callback,
+                            bool newly_loaded, Profile* profile) {
+                           std::move(callback).Run(profile,
+                                                   profile && newly_loaded);
+                         },
+                         std::move(callback), newly_loaded));
+}
+
 void ProfileManager::CreateProfileAsync(
     const base::FilePath& profile_path,
     base::OnceCallback<void(Profile*)> initialized_callback,
