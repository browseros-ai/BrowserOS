diff --git a/chrome/browser/devtools/devtools_browser_context_manager.h b/chrome/browser/devtools/devtools_browser_context_manager.h
index 1898b7eec71c0810afc4a75074aa3178f19067b4..ab41f83e78e474dab6be21473eddf21f4f4b5b66 100644
--- a/chrome/browser/devtools/devtools_browser_context_manager.h
+++ b/chrome/browser/devtools/devtools_browser_context_manager.h
@@ -5,23 +5,51 @@
 #ifndef CHROME_BROWSER_DEVTOOLS_DEVTOOLS_BROWSER_CONTEXT_MANAGER_H_
 #define CHROME_BROWSER_DEVTOOLS_DEVTOOLS_BROWSER_CONTEXT_MANAGER_H_
 
+#include <optional>
+#include <string>
+#include <vector>
+
 #include "base/containers/flat_map.h"
+#include "base/containers/flat_set.h"
+#include "base/files/file_path.h"
+#include "base/functional/callback.h"
 #include "base/memory/raw_ptr.h"
 #include "base/memory/weak_ptr.h"
 #include "base/no_destructor.h"
 #include "base/scoped_multi_source_observation.h"
 #include "base/scoped_observation.h"
+#include "base/timer/timer.h"
 #include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/profiles/profile_attributes_storage_observer.h"
+#include "chrome/browser/profiles/profile_manager_observer.h"
 #include "chrome/browser/profiles/profile_observer.h"
 #include "chrome/browser/ui/browser_window/public/browser_collection_observer.h"
 #include "content/public/browser/devtools_manager_delegate.h"
 
 class BrowserWindowInterface;
 class GlobalBrowserCollection;
+class ProfileAttributesStorage;
+class ProfileManager;
+class ScopedProfileKeepAlive;
 
+// Owns DevTools-created OTR contexts and the separate process-lifetime saved
+// regular-profile catalog. Saved handles survive context unloading, while live
+// UniqueIds route only to currently loaded, eligible contexts.
 class DevToolsBrowserContextManager : public BrowserCollectionObserver,
-                                      public ProfileObserver {
+                                      public ProfileObserver,
+                                      public ProfileAttributesStorageObserver,
+                                      public ProfileManagerObserver {
  public:
+  struct SavedProfileInfo {
+    std::string browser_profile_id;
+    std::string name;
+    std::string directory;
+    bool loaded = false;
+    std::optional<std::string> browser_context_id;
+  };
+  using LoadProfileCallback = base::OnceCallback<
+      void(std::optional<SavedProfileInfo>, bool, const std::string&)>;
+
   static DevToolsBrowserContextManager& GetInstance();
 
   DevToolsBrowserContextManager(const DevToolsBrowserContextManager&) = delete;
@@ -29,6 +57,13 @@ class DevToolsBrowserContextManager : public BrowserCollectionObserver,
       const DevToolsBrowserContextManager&) = delete;
 
   Profile* GetProfileById(const std::string& browser_context_id);
+  std::vector<SavedProfileInfo> GetProfiles();
+  std::optional<std::string> GetBrowserProfileId(Profile* profile);
+  // Browser and Chrome Target creation share this policy gate, since both
+  // handlers can now address a loaded regular profile without activating it.
+  bool CanCreateRegularProfileWindow(Profile* profile);
+  void LoadProfile(const std::string& browser_profile_id,
+                   LoadProfileCallback callback);
   std::vector<base::WeakPtr<content::BrowserContext>> GetBrowserContexts();
   content::BrowserContext* GetDefaultBrowserContext();
   content::BrowserContext* CreateBrowserContext();
@@ -47,6 +82,40 @@ class DevToolsBrowserContextManager : public BrowserCollectionObserver,
   // ProfileObserver:
   void OnProfileWillBeDestroyed(Profile* profile) override;
 
+  // Storage notifications invalidate immediately, even if a directory is
+  // removed and re-added between two CDP requests.
+  using ProfileManagerObserver::OnProfileAdded;
+  void OnProfileAdded(const base::FilePath& profile_path) override;
+  void OnProfileWillBeRemoved(const base::FilePath& profile_path) override;
+  void OnProfileWasRemoved(const base::FilePath& profile_path,
+                           const std::u16string& profile_name) override;
+  void OnProfileIsOmittedChanged(const base::FilePath& profile_path) override;
+  void OnProfileMarkedForPermanentDeletion(Profile* profile) override;
+  void OnProfileManagerDestroying() override;
+
+  bool EnsureProfileManagerObservation();
+  bool IsEligibleProfilePath(const base::FilePath& path);
+  SavedProfileInfo BuildSavedProfileInfo(const base::FilePath& path);
+  void InvalidateSavedProfile(const base::FilePath& path);
+  void OnSavedProfileLoaded(const std::string& browser_profile_id,
+                            Profile* profile,
+                            bool newly_loaded);
+  void FinishProfileLoad(const std::string& browser_profile_id,
+                         const std::string& error);
+  void OnProfileLoadTimeout(const std::string& browser_profile_id);
+  void CompleteProfileLoadWaiters(const std::string& browser_profile_id,
+                                  const std::string& error);
+
+  // One underlying initialization per saved identity, with a bounded number of
+  // waiters and a deadline. The timer releases CDP callbacks even if Chromium's
+  // deferred loader never reaches its completion callback during shutdown.
+  struct PendingProfileLoad {
+    std::vector<LoadProfileCallback> callbacks;
+    base::OneShotTimer deadline;
+    bool callbacks_completed = false;
+    bool newly_loaded = false;
+  };
+
   // Idempotent function ensuring the `profile` is not more observed, and
   // `otr_profiles_` is updated accordingly.
   void StopObservingProfileIfAny(Profile* profile);
@@ -59,6 +128,20 @@ class DevToolsBrowserContextManager : public BrowserCollectionObserver,
   base::flat_map<std::string, content::DevToolsManagerDelegate::DisposeCallback>
       pending_context_disposals_;
 
+  base::ScopedObservation<ProfileManager, ProfileManagerObserver>
+      profile_manager_observation_{this};
+  base::ScopedObservation<ProfileAttributesStorage,
+                          ProfileAttributesStorageObserver>
+      profile_attributes_observation_{this};
+  base::flat_map<base::FilePath, std::string> saved_profile_ids_;
+  base::flat_set<base::FilePath> retiring_profile_paths_;
+  base::flat_map<std::string, std::unique_ptr<PendingProfileLoad>>
+      profile_loads_;
+  // Explicitly loaded profiles remain usable without a window. These leases
+  // are bounded by catalog deletion or browser shutdown, not by window focus.
+  base::flat_map<base::FilePath, std::unique_ptr<ScopedProfileKeepAlive>>
+      loaded_profile_keep_alives_;
+
   base::WeakPtrFactory<DevToolsBrowserContextManager> weak_factory_{this};
 };
 
