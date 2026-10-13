diff --git a/chrome/browser/profiles/profile_manager.cc b/chrome/browser/profiles/profile_manager.cc
index 5856f418248ac92b8e695c1bfe54ad1ab13b1360..fc7f22eb39dd9cd8c9bc41287737b0219a014ac3 100644
--- a/chrome/browser/profiles/profile_manager.cc
+++ b/chrome/browser/profiles/profile_manager.cc
@@ -832,10 +832,48 @@ bool ProfileManager::LoadProfileByPath(const base::FilePath& profile_path,
   return true;
 }
 
+void ProfileManager::LoadExistingProfileByPath(
+    const base::FilePath& profile_path,
+    base::OnceCallback<void(Profile*, bool)> callback) {
+  DCHECK_CURRENTLY_ON(BrowserThread::UI);
+  // Decide ownership only after startup/unloading deferrals. Counting a
+  // preflight request as the owner could report a new load when another
+  // Chromium caller initializes the profile before the deferred call runs.
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
+  if (!GetProfileAttributesStorage().GetProfileAttributesWithPath(
+          profile_path)) {
+    std::move(callback).Run(nullptr, false);
+    return;
+  }
+  const bool newly_loaded = !profiles_info_.contains(profile_path);
+  CreateProfileAsync(profile_path,
+                     base::BindOnce(
+                         [](base::OnceCallback<void(Profile*, bool)> callback,
+                            bool newly_loaded, Profile* profile) {
+                           std::move(callback).Run(profile,
+                                                   profile && newly_loaded);
+                         },
+                         std::move(callback), newly_loaded),
+                     {}, /*create_if_missing=*/false);
+}
+
 void ProfileManager::CreateProfileAsync(
     const base::FilePath& profile_path,
     base::OnceCallback<void(Profile*)> initialized_callback,
-    base::OnceCallback<void(Profile*)> created_callback) {
+    base::OnceCallback<void(Profile*)> created_callback,
+    bool create_if_missing) {
   DCHECK_CURRENTLY_ON(BrowserThread::UI);
   TRACE_EVENT1("browser,startup", "ProfileManager::CreateProfileAsync",
                "profile_path", profile_path.AsUTF8Unsafe());
@@ -846,7 +884,7 @@ void ProfileManager::CreateProfileAsync(
     deferred_asynchronous_loads_.push_back(base::BindOnce(
         &ProfileManager::CreateProfileAsync, base::Unretained(this),
         profile_path, std::move(initialized_callback),
-        std::move(created_callback)));
+        std::move(created_callback), create_if_missing));
     return;
   }
 
@@ -857,11 +895,14 @@ void ProfileManager::CreateProfileAsync(
     pending_it->second.push_back(base::BindOnce(
         &ProfileManager::CreateProfileAsync, weak_factory_.GetWeakPtr(),
         profile_path, std::move(initialized_callback),
-        std::move(created_callback)));
+        std::move(created_callback), create_if_missing));
     return;
   }
 
-  if (!CanCreateProfileAtPath(profile_path)) {
+  if (!CanCreateProfileAtPath(profile_path) ||
+      (!create_if_missing &&
+       !GetProfileAttributesStorage().GetProfileAttributesWithPath(
+           profile_path))) {
     if (!initialized_callback.is_null()) {
       std::move(initialized_callback).Run(nullptr);
     }
@@ -876,7 +917,19 @@ void ProfileManager::CreateProfileAsync(
     info = iter->second.get();
   } else {
     // Initiate asynchronous creation process.
-    info = RegisterOwnedProfile(CreateProfileAsyncHelper(profile_path));
+    std::unique_ptr<Profile> profile =
+        create_if_missing
+            ? CreateProfileAsyncHelper(profile_path)
+            : Profile::CreateProfile(profile_path, this,
+                                     Profile::CreateMode::kAsynchronous,
+                                     /*create_if_missing=*/false);
+    if (!profile) {
+      if (!initialized_callback.is_null()) {
+        std::move(initialized_callback).Run(nullptr);
+      }
+      return;
+    }
+    info = RegisterOwnedProfile(std::move(profile));
   }
 
   // Call or enqueue the callback.
@@ -1095,7 +1148,8 @@ void ProfileManager::CreateMultiProfileAsync(
                      base::BindOnce(&ProfileManager::CreateProfileAsync,
                                     profile_manager->weak_factory_.GetWeakPtr(),
                                     new_path, std::move(initialized_callback),
-                                    std::move(created_callback))));
+                                    std::move(created_callback),
+                                    /*create_if_missing=*/true)));
 }
 #endif  // !BUILDFLAG(IS_ANDROID)
 
