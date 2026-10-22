diff --git a/chrome/browser/devtools/devtools_browser_context_manager.cc b/chrome/browser/devtools/devtools_browser_context_manager.cc
index 8137e5f95157d4fcb045f325d6d4b0bde11b2b37..a1fecbb87fe4c425e2e1938938866ec4a61ee5ba 100644
--- a/chrome/browser/devtools/devtools_browser_context_manager.cc
+++ b/chrome/browser/devtools/devtools_browser_context_manager.cc
@@ -4,28 +4,43 @@
 
 #include "chrome/browser/devtools/devtools_browser_context_manager.h"
 
+#include "base/command_line.h"
 #include "base/functional/bind.h"
 #include "base/functional/callback_helpers.h"
 #include "base/no_destructor.h"
+#include "base/strings/utf_string_conversions.h"
 #include "base/task/single_thread_task_runner.h"
+#include "base/unguessable_token.h"
 #include "build/build_config.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/profiles/keep_alive/profile_keep_alive_types.h"
+#include "chrome/browser/profiles/keep_alive/scoped_profile_keep_alive.h"
+#include "chrome/browser/profiles/profile_attributes_entry.h"
+#include "chrome/browser/profiles/profile_attributes_storage.h"
 #include "chrome/browser/profiles/profile_destroyer.h"
 #include "chrome/browser/profiles/profile_manager.h"
+#include "chrome/browser/profiles/profiles_state.h"
+#include "chrome/common/chrome_constants.h"
 #if BUILDFLAG(IS_ANDROID)
 #include "chrome/browser/android/tab_android.h"
 #include "chrome/browser/ui/android/tab_model/tab_model.h"
 #include "chrome/browser/ui/android/tab_model/tab_model_list.h"
 #else
 #include "chrome/browser/lifetime/application_lifetime_desktop.h"
+#include "chrome/browser/profiles/nuke_profile_directory_utils.h"
 #endif  // BUILDFLAG(IS_ANDROID)
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface_iterator.h"
+#include "chrome/browser/ui/browser_window/public/create_browser_window.h"
 #include "chrome/browser/ui/browser_window/public/global_browser_collection.h"
 #include "ui/base/base_window.h"
 
 namespace {
 
 const int64_t kDestroyProfileTimeoutSeconds = 60;
+constexpr size_t kMaxPendingProfileLoads = 16;
+constexpr size_t kMaxProfileLoadWaiters = 64;
+constexpr base::TimeDelta kProfileLoadTimeout = base::Seconds(30);
 
 void DestroyOTRProfileWhenAppropriate(base::WeakPtr<Profile> weak_profile) {
   if (Profile* profile = weak_profile.get()) {
@@ -65,10 +80,19 @@ DevToolsBrowserContextManager& DevToolsBrowserContextManager::GetInstance() {
 
 Profile* DevToolsBrowserContextManager::GetProfileById(
     const std::string& context_id) {
-  Profile* default_profile =
-      ProfileManager::GetLastUsedProfile()->GetOriginalProfile();
-  if (context_id == default_profile->UniqueId()) {
-    return default_profile;
+  // Both root CDP and Chrome's Target handler use this resolver. Never load a
+  // profile to resolve a live ID, and never confuse its directory label with
+  // BrowserContext::UniqueId(). OTR enumeration/disposal stays in
+  // otr_profiles_.
+  if (EnsureProfileManagerObservation()) {
+    for (Profile* profile :
+         profile_manager_observation_.GetSource()->GetLoadedProfiles()) {
+      if (profile->IsRegularProfile() && !profile->IsOffTheRecord() &&
+          context_id == profile->UniqueId() &&
+          IsEligibleProfilePath(profile->GetPath())) {
+        return profile;
+      }
+    }
   }
   auto it = otr_profiles_.find(context_id);
   if (it == otr_profiles_.end())
@@ -76,6 +100,311 @@ Profile* DevToolsBrowserContextManager::GetProfileById(
   return it->second;
 }
 
+bool DevToolsBrowserContextManager::EnsureProfileManagerObservation() {
+  if (!g_browser_process || g_browser_process->IsShuttingDown() ||
+      !g_browser_process->profile_manager()) {
+    return false;
+  }
+  if (!profile_manager_observation_.IsObserving()) {
+    ProfileManager* manager = g_browser_process->profile_manager();
+    profile_manager_observation_.Observe(manager);
+    profile_attributes_observation_.Observe(
+        &manager->GetProfileAttributesStorage());
+  }
+  return true;
+}
+
+bool DevToolsBrowserContextManager::IsEligibleProfilePath(
+    const base::FilePath& path) {
+  if (!EnsureProfileManagerObservation()) {
+    return false;
+  }
+  ProfileManager* manager = profile_manager_observation_.GetSource();
+  auto* entry =
+      manager->GetProfileAttributesStorage().GetProfileAttributesWithPath(path);
+  if (!entry || entry->IsOmitted() || retiring_profile_paths_.contains(path) ||
+      path.DirName() != manager->user_data_dir() ||
+      path == ProfileManager::GetGuestProfilePath() ||
+      path.BaseName().value() == chrome::kSystemProfileDir) {
+    return false;
+  }
+#if !BUILDFLAG(IS_ANDROID)
+  if (IsProfileDirectoryMarkedForDeletion(path)) {
+    return false;
+  }
+#endif
+  Profile* profile = manager->GetProfileByPath(path);
+  return !profile ||
+         (profile->IsRegularProfile() && !profile->IsOffTheRecord());
+}
+
+DevToolsBrowserContextManager::SavedProfileInfo
+DevToolsBrowserContextManager::BuildSavedProfileInfo(
+    const base::FilePath& path) {
+  ProfileManager* manager = profile_manager_observation_.GetSource();
+  auto* entry =
+      manager->GetProfileAttributesStorage().GetProfileAttributesWithPath(path);
+  auto [it, inserted] = saved_profile_ids_.try_emplace(path);
+  if (inserted) {
+    it->second = base::UnguessableToken::Create().ToString();
+  }
+  SavedProfileInfo info;
+  info.browser_profile_id = it->second;
+  info.name = base::UTF16ToUTF8(entry->GetLocalProfileName());
+  info.directory = path.BaseName().AsUTF8Unsafe();
+  if (Profile* profile = manager->GetProfileByPath(path)) {
+    info.loaded = true;
+    info.browser_context_id = profile->UniqueId();
+  }
+  return info;
+}
+
+std::vector<DevToolsBrowserContextManager::SavedProfileInfo>
+DevToolsBrowserContextManager::GetProfiles() {
+  std::vector<SavedProfileInfo> profiles;
+  if (!EnsureProfileManagerObservation()) {
+    return profiles;
+  }
+  for (auto* entry : profile_manager_observation_.GetSource()
+                         ->GetProfileAttributesStorage()
+                         .GetAllProfilesAttributes()) {
+    if (IsEligibleProfilePath(entry->GetPath())) {
+      profiles.push_back(BuildSavedProfileInfo(entry->GetPath()));
+    }
+  }
+  return profiles;
+}
+
+std::optional<std::string> DevToolsBrowserContextManager::GetBrowserProfileId(
+    Profile* profile) {
+  // OTR contexts share their original profile's path, so the live Profile
+  // identity must be checked before consulting the saved catalog by path.
+  if (!profile || !profile->IsRegularProfile() || profile->IsOffTheRecord() ||
+      !IsEligibleProfilePath(profile->GetPath())) {
+    return std::nullopt;
+  }
+  return BuildSavedProfileInfo(profile->GetPath()).browser_profile_id;
+}
+
+bool DevToolsBrowserContextManager::CanCreateRegularProfileWindow(
+    Profile* profile) {
+  if (!GetBrowserProfileId(profile)) {
+    return false;
+  }
+  auto* entry = profile_manager_observation_.GetSource()
+                    ->GetProfileAttributesStorage()
+                    .GetProfileAttributesWithPath(profile->GetPath());
+  // Chromium's creation-status check covers incognito policy, shutdown and
+  // deletion, but does not cover the saved profile's sign-in lock. Apply all
+  // gates before either handler reaches navigation or CHECK-bearing creation.
+  return !entry->IsSigninRequired() &&
+         !profiles::IsGuestModeRequested(
+             *base::CommandLine::ForCurrentProcess(),
+             g_browser_process->local_state(), false) &&
+         GetBrowserWindowCreationStatusForProfile(*profile) ==
+             BrowserWindowInterface::CreationStatus::kOk;
+}
+
+void DevToolsBrowserContextManager::LoadProfile(
+    const std::string& browser_profile_id,
+    LoadProfileCallback callback) {
+  if (!EnsureProfileManagerObservation()) {
+    std::move(callback).Run(std::nullopt, false, "Profile manager unavailable");
+    return;
+  }
+  base::FilePath path;
+  for (const auto& [candidate, id] : saved_profile_ids_) {
+    if (id == browser_profile_id) {
+      path = candidate;
+      break;
+    }
+  }
+  if (path.empty() || !IsEligibleProfilePath(path)) {
+    std::move(callback).Run(std::nullopt, false,
+                            "Unknown or stale browserProfileId");
+    return;
+  }
+  auto pending = profile_loads_.find(browser_profile_id);
+  if (pending != profile_loads_.end()) {
+    if (pending->second->callbacks_completed) {
+      std::move(callback).Run(std::nullopt, false,
+                              "Previous profile load is still pending");
+    } else if (pending->second->callbacks.size() >= kMaxProfileLoadWaiters) {
+      std::move(callback).Run(std::nullopt, false,
+                              "Too many profile load waiters");
+    } else {
+      pending->second->callbacks.push_back(std::move(callback));
+    }
+    return;
+  }
+  if (Profile* profile =
+          profile_manager_observation_.GetSource()->GetProfileByPath(path)) {
+    if (!loaded_profile_keep_alives_.contains(path)) {
+      auto keep_alive = ScopedProfileKeepAlive::TryAcquire(
+          profile, ProfileKeepAliveOrigin::kRemoteDebugging);
+      if (!keep_alive) {
+        std::move(callback).Run(std::nullopt, false, "Profile is unloading");
+        return;
+      }
+      loaded_profile_keep_alives_[path] = std::move(keep_alive);
+    }
+    std::move(callback).Run(BuildSavedProfileInfo(path), false, "");
+    return;
+  }
+  if (profile_loads_.size() >= kMaxPendingProfileLoads) {
+    std::move(callback).Run(std::nullopt, false,
+                            "Too many pending profile loads");
+    return;
+  }
+  auto load = std::make_unique<PendingProfileLoad>();
+  load->callbacks.push_back(std::move(callback));
+  load->deadline.Start(
+      FROM_HERE, kProfileLoadTimeout,
+      base::BindOnce(&DevToolsBrowserContextManager::OnProfileLoadTimeout,
+                     weak_factory_.GetWeakPtr(), browser_profile_id));
+  profile_loads_[browser_profile_id] = std::move(load);
+  profile_manager_observation_.GetSource()->LoadExistingProfileByPath(
+      path, base::BindOnce(&DevToolsBrowserContextManager::OnSavedProfileLoaded,
+                           weak_factory_.GetWeakPtr(), browser_profile_id));
+}
+
+void DevToolsBrowserContextManager::OnSavedProfileLoaded(
+    const std::string& browser_profile_id,
+    Profile* profile,
+    bool newly_loaded) {
+  // A timeout/removal already completed its callbacks. Late completions must
+  // never revive that identity or borrow a replacement at the same path.
+  if (!profile_loads_.contains(browser_profile_id)) {
+    return;
+  }
+  if (profile_loads_.at(browser_profile_id)->callbacks_completed) {
+    FinishProfileLoad(browser_profile_id,
+                      "Profile load is no longer requested");
+    return;
+  }
+  if (!profile || GetBrowserProfileId(profile) != browser_profile_id) {
+    FinishProfileLoad(browser_profile_id,
+                      "Existing profile could not be loaded");
+    return;
+  }
+  auto keep_alive = ScopedProfileKeepAlive::TryAcquire(
+      profile, ProfileKeepAliveOrigin::kRemoteDebugging);
+  if (!keep_alive) {
+    FinishProfileLoad(browser_profile_id, "Profile is unloading");
+    return;
+  }
+  loaded_profile_keep_alives_[profile->GetPath()] = std::move(keep_alive);
+  profile_loads_.at(browser_profile_id)->newly_loaded = newly_loaded;
+  FinishProfileLoad(browser_profile_id, "");
+}
+
+void DevToolsBrowserContextManager::OnProfileLoadTimeout(
+    const std::string& browser_profile_id) {
+  CompleteProfileLoadWaiters(browser_profile_id, "Profile load timed out");
+}
+
+void DevToolsBrowserContextManager::CompleteProfileLoadWaiters(
+    const std::string& browser_profile_id,
+    const std::string& error) {
+  auto it = profile_loads_.find(browser_profile_id);
+  if (it == profile_loads_.end()) {
+    return;
+  }
+  // Timeout and catalog invalidation both complete CDP waiters immediately,
+  // but cannot cancel ProfileManager's callback. Keep its slot until that
+  // callback finishes so repeated invalidations cannot exceed the load limit.
+  it->second->callbacks_completed = true;
+  it->second->deadline.Stop();
+  auto callbacks = std::move(it->second->callbacks);
+  for (auto& callback : callbacks) {
+    std::move(callback).Run(std::nullopt, false, error);
+  }
+}
+
+void DevToolsBrowserContextManager::FinishProfileLoad(
+    const std::string& browser_profile_id,
+    const std::string& error) {
+  auto pending = profile_loads_.find(browser_profile_id);
+  if (pending == profile_loads_.end()) {
+    return;
+  }
+  auto load = std::move(pending->second);
+  profile_loads_.erase(pending);
+  load->deadline.Stop();
+  std::optional<SavedProfileInfo> info;
+  if (error.empty()) {
+    for (const auto& [path, id] : saved_profile_ids_) {
+      if (id == browser_profile_id && IsEligibleProfilePath(path)) {
+        info = BuildSavedProfileInfo(path);
+        break;
+      }
+    }
+  }
+  // Erase pending state before replying: sending a CDP response can re-enter
+  // dispatch. Only a caller that actually initiated ProfileManager's load
+  // reports true; joining a load started elsewhere in Chromium reports false.
+  for (size_t i = 0; i < load->callbacks.size(); ++i) {
+    std::move(load->callbacks[i])
+        .Run(info, info.has_value() && load->newly_loaded && i == 0,
+             error.empty() && !info ? "Profile disappeared" : error);
+  }
+}
+
+void DevToolsBrowserContextManager::InvalidateSavedProfile(
+    const base::FilePath& path) {
+  auto it = saved_profile_ids_.find(path);
+  if (it != saved_profile_ids_.end()) {
+    std::string id = it->second;
+    saved_profile_ids_.erase(it);
+    CompleteProfileLoadWaiters(id, "Saved profile was removed or omitted");
+  }
+  loaded_profile_keep_alives_.erase(path);
+}
+
+void DevToolsBrowserContextManager::OnProfileAdded(const base::FilePath& path) {
+  retiring_profile_paths_.erase(path);
+}
+
+void DevToolsBrowserContextManager::OnProfileWillBeRemoved(
+    const base::FilePath& path) {
+  retiring_profile_paths_.insert(path);
+  InvalidateSavedProfile(path);
+}
+
+void DevToolsBrowserContextManager::OnProfileWasRemoved(
+    const base::FilePath& path,
+    const std::u16string& profile_name) {
+  // Once the catalog entry is gone, eligibility itself prevents reuse. Keep
+  // retirement markers only across the synchronous removal notification.
+  retiring_profile_paths_.erase(path);
+}
+
+void DevToolsBrowserContextManager::OnProfileIsOmittedChanged(
+    const base::FilePath& path) {
+  if (!IsEligibleProfilePath(path)) {
+    InvalidateSavedProfile(path);
+  }
+}
+
+void DevToolsBrowserContextManager::OnProfileMarkedForPermanentDeletion(
+    Profile* profile) {
+  OnProfileWillBeRemoved(profile->GetPath());
+}
+
+void DevToolsBrowserContextManager::OnProfileManagerDestroying() {
+  // This singleton outlives BrowserProcess. Detach while both observed owners
+  // still exist, and finish bounded requests before their profiles disappear.
+  profile_attributes_observation_.Reset();
+  profile_manager_observation_.Reset();
+  saved_profile_ids_.clear();
+  retiring_profile_paths_.clear();
+  loaded_profile_keep_alives_.clear();
+  while (!profile_loads_.empty()) {
+    std::string id = profile_loads_.begin()->first;
+    FinishProfileLoad(id, "Browser is shutting down");
+  }
+}
+
 content::BrowserContext* DevToolsBrowserContextManager::CreateBrowserContext() {
   Profile* original_profile =
       ProfileManager::GetLastUsedProfile()->GetOriginalProfile();
