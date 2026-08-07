diff --git a/chrome/browser/extensions/extension_management.cc b/chrome/browser/extensions/extension_management.cc
index bea1864156f76066fe88de154d06ef5070f6f0ac..9eaef771c6bf35b81c9786c4bb0150a3db05fc2c 100644
--- a/chrome/browser/extensions/extension_management.cc
+++ b/chrome/browser/extensions/extension_management.cc
@@ -25,6 +25,8 @@
 #include "base/values.h"
 #include "base/version.h"
 #include "build/chromeos_buildflags.h"
+#include "chrome/browser/browser_features.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/enterprise/util/managed_browser_utils.h"
 #include "chrome/browser/extensions/cws_info_service_factory.h"
 #include "chrome/browser/extensions/extension_management_constants.h"
@@ -186,7 +188,7 @@ bool ExtensionManagement::ExtensionsEnabledForDesktopAndroid() const {
     std::string domain = gaia::ExtractDomainName(user_name);
     if (domain == "google.com" || domain == "managedchrome.com") {
       return base::FeatureList::IsEnabled(
-              extensions_features::kEnableExtensionsForCorpDesktopAndroid);
+          extensions_features::kEnableExtensionsForCorpDesktopAndroid);
     }
   }
   return true;
@@ -208,13 +210,15 @@ ManagedInstallationMode ExtensionManagement::GetInstallationMode(
 
   // Check per-extension installation mode setting first.
   auto* setting = GetSettingsForId(extension_id);
-  if (setting)
+  if (setting) {
     return setting->installation_mode;
+  }
   // Check per-update-url installation mode setting.
   if (!update_url.empty()) {
     auto iter_update_url = settings_by_update_url_.find(update_url);
-    if (iter_update_url != settings_by_update_url_.end())
+    if (iter_update_url != settings_by_update_url_.end()) {
       return iter_update_url->second->installation_mode;
+    }
   }
   // Fall back to default installation mode setting.
   return default_settings_->installation_mode;
@@ -272,6 +276,16 @@ bool ExtensionManagement::IsUpdateUrlOverridden(const ExtensionId& id) {
 }
 
 GURL ExtensionManagement::GetEffectiveUpdateURL(const Extension& extension) {
+  // BrowserOS: catalog extensions always update from the product manifest,
+  // even when the CRX omits update_url (as BrowserClaw does). Choose the
+  // channel here so mid-session flips apply on the next update check.
+  if (browseros::IsActiveBrowserOSExtension(extension.id())) {
+    return GURL(
+        base::FeatureList::IsEnabled(features::kBrowserOsAlphaFeatures)
+            ? browseros::kBrowserOSAlphaUpdateUrl
+            : browseros::kBrowserOSUpdateUrl);
+  }
+
   if (IsUpdateUrlOverridden(extension.id())) {
     DCHECK(!extension.was_installed_by_default())
         << "Update URL should not be overridden for default-installed "
@@ -320,8 +334,9 @@ bool ExtensionManagement::IsInstallationExplicitlyBlocked(
     const ExtensionId& id) {
   auto* setting = GetSettingsForId(id);
   // No settings explicitly specified for |id|.
-  if (setting == nullptr)
+  if (setting == nullptr) {
     return false;
+  }
   // Checks if the extension is listed as blocked or removed.
   ManagedInstallationMode mode = setting->installation_mode;
   return mode == ManagedInstallationMode::kBlocked ||
@@ -332,13 +347,15 @@ bool ExtensionManagement::IsOffstoreInstallAllowed(
     const GURL& url,
     const GURL& referrer_url) const {
   // No allowed install sites specified, disallow by default.
-  if (!global_settings_->install_sources.has_value())
+  if (!global_settings_->install_sources.has_value()) {
     return false;
+  }
 
   const URLPatternSet& url_patterns = *global_settings_->install_sources;
 
-  if (!url_patterns.MatchesURL(url))
+  if (!url_patterns.MatchesURL(url)) {
     return false;
+  }
 
   // The referrer URL must also be allowlisted, unless the URL has the file
   // scheme (there's no referrer for those URLs).
@@ -352,12 +369,14 @@ bool ExtensionManagement::IsAllowedManifestType(
   // If a managed theme has been set for the current profile, theme extension
   // installations are not allowed.
   if (manifest_type == Manifest::Type::TYPE_THEME &&
-      ThemeServiceFactory::GetForProfile(profile_)->UsingPolicyTheme())
+      ThemeServiceFactory::GetForProfile(profile_)->UsingPolicyTheme()) {
     return false;
+  }
 #endif  // BUILDFLAG(ENABLE_EXTENSIONS)
 
-  if (!global_settings_->allowed_types.has_value())
+  if (!global_settings_->allowed_types.has_value()) {
     return true;
+  }
   const std::vector<Manifest::Type>& allowed_types =
       *global_settings_->allowed_types;
   return std::ranges::contains(allowed_types, manifest_type);
@@ -571,8 +590,9 @@ APIPermissionSet ExtensionManagement::GetBlockedAPIPermissions(
 
   // Fetch per-update-url blocked permissions setting.
   auto iter_update_url = settings_by_update_url_.end();
-  if (!update_url.empty())
+  if (!update_url.empty()) {
     iter_update_url = settings_by_update_url_.find(update_url);
+  }
 
   if (setting && iter_update_url != settings_by_update_url_.end()) {
     // Blocked permissions setting are specified in both per-extension and
@@ -584,10 +604,12 @@ APIPermissionSet ExtensionManagement::GetBlockedAPIPermissions(
     return merged;
   }
   // Check whether if in one of them, setting is specified.
-  if (setting)
+  if (setting) {
     return setting->blocked_permissions.Clone();
-  if (iter_update_url != settings_by_update_url_.end())
+  }
+  if (iter_update_url != settings_by_update_url_.end()) {
     return iter_update_url->second->blocked_permissions.Clone();
+  }
   // Fall back to the default blocked permissions setting.
   return default_settings_->blocked_permissions.Clone();
 }
@@ -603,16 +625,18 @@ const URLPatternSet& ExtensionManagement::GetDefaultPolicyAllowedHosts() const {
 const URLPatternSet& ExtensionManagement::GetPolicyBlockedHosts(
     const Extension* extension) {
   auto* setting = GetSettingsForId(extension->id());
-  if (setting)
+  if (setting) {
     return setting->policy_blocked_hosts;
+  }
   return default_settings_->policy_blocked_hosts;
 }
 
 const URLPatternSet& ExtensionManagement::GetPolicyAllowedHosts(
     const Extension* extension) {
   auto* setting = GetSettingsForId(extension->id());
-  if (setting)
+  if (setting) {
     return setting->policy_allowed_hosts;
+  }
   return default_settings_->policy_allowed_hosts;
 }
 
@@ -643,8 +667,9 @@ bool ExtensionManagement::IsPermissionSetAllowed(
     const PermissionSet& perms) {
   for (const APIPermission* blocked_api :
        GetBlockedAPIPermissions(extension_id, update_url)) {
-    if (perms.HasAPIPermission(blocked_api->id()))
+    if (perms.HasAPIPermission(blocked_api->id())) {
       return false;
+    }
   }
   return true;
 }
@@ -652,8 +677,9 @@ bool ExtensionManagement::IsPermissionSetAllowed(
 const std::string ExtensionManagement::BlockedInstallMessage(
     const ExtensionId& id) {
   auto* setting = GetSettingsForId(id);
-  if (setting)
+  if (setting) {
     return setting->blocked_install_message;
+  }
   return default_settings_->blocked_install_message;
 }
 
@@ -664,6 +690,14 @@ ExtensionIdSet ExtensionManagement::GetForcePinnedList() const {
       force_pinned_list.insert(entry.first);
     }
   }
+
+  // Always force-pin BrowserOS extensions that are marked pinned.
+  for (const auto& extension_id : browseros::GetActiveBrowserOSExtensionIds()) {
+    if (browseros::IsBrowserOSPinnedExtension(extension_id)) {
+      force_pinned_list.insert(extension_id);
+    }
+  }
+
   return force_pinned_list;
 }
 
@@ -685,13 +719,15 @@ bool ExtensionManagement::CheckMinimumVersion(const Extension* extension,
                                               std::string* required_version) {
   auto* setting = GetSettingsForId(extension->id());
   // If there are no minimum version required for |extension|, return true.
-  if (!setting || !setting->minimum_version_required)
+  if (!setting || !setting->minimum_version_required) {
     return true;
+  }
   bool meets_requirement =
       extension->version().CompareTo(*setting->minimum_version_required) >= 0;
   // Output a human readable version string for prompting if necessary.
-  if (!meets_requirement && required_version)
+  if (!meets_requirement && required_version) {
     *required_version = setting->minimum_version_required->GetString();
+  }
   return meets_requirement;
 }
 
@@ -747,28 +783,34 @@ void ExtensionManagement::Refresh() {
     // Settings from new preference have higher priority over legacy ones.
     const base::ListValue* list_value =
         subdict->FindList(schema_constants::kInstallSources);
-    if (list_value)
+    if (list_value) {
       install_sources_pref = list_value;
+    }
 
     list_value = subdict->FindList(schema_constants::kAllowedTypes);
-    if (list_value)
+    if (list_value) {
       allowed_types_pref = list_value;
+    }
   }
 
   // Parse legacy preferences.
   if (allowed_list_pref) {
     for (const auto& entry : *allowed_list_pref) {
-      if (entry.is_string() && crx_file::id_util::IdIsValid(entry.GetString()))
+      if (entry.is_string() &&
+          crx_file::id_util::IdIsValid(entry.GetString())) {
         AccessById(entry.GetString())->installation_mode =
             ManagedInstallationMode::kAllowed;
+      }
     }
   }
 
   if (denied_list_pref) {
     for (const auto& entry : *denied_list_pref) {
-      if (entry.is_string() && crx_file::id_util::IdIsValid(entry.GetString()))
+      if (entry.is_string() &&
+          crx_file::id_util::IdIsValid(entry.GetString())) {
         AccessById(entry.GetString())->installation_mode =
             ManagedInstallationMode::kBlocked;
+      }
     }
   }
 
@@ -801,8 +843,9 @@ void ExtensionManagement::Refresh() {
       } else if (entry.is_string()) {
         Manifest::Type manifest_type =
             schema_constants::GetManifestType(entry.GetString());
-        if (manifest_type != Manifest::TYPE_UNKNOWN)
+        if (manifest_type != Manifest::TYPE_UNKNOWN) {
           global_settings_->allowed_types->push_back(manifest_type);
+        }
       }
     }
   }
@@ -833,11 +876,13 @@ void ExtensionManagement::Refresh() {
         std::move(installed_extension_ids));
 
     for (auto iter : *dict_pref) {
-      if (iter.first == schema_constants::kWildcard)
+      if (iter.first == schema_constants::kWildcard) {
         continue;
+      }
       const base::DictValue* subdict = iter.second.GetIfDict();
-      if (!subdict)
+      if (!subdict) {
         continue;
+      }
       std::optional<std::string_view> remainder =
           base::RemovePrefix(iter.first, schema_constants::kUpdateUrlPrefix);
       if (remainder) {
@@ -893,8 +938,9 @@ void ExtensionManagement::Refresh() {
           internal::IndividualSettings* by_id = AccessById(extension_id);
           const bool included_in_forcelist =
               by_id->installation_mode == ManagedInstallationMode::kForced;
-          if (!ParseById(extension_id, *subdict))
+          if (!ParseById(extension_id, *subdict)) {
             continue;
+          }
 
           // If applying the ExtensionSettings policy changes installation mode
           // from force-installed to anything else, the extension might not get
@@ -915,8 +961,9 @@ void ExtensionManagement::Refresh() {
 bool ExtensionManagement::ParseById(const std::string& extension_id,
                                     const base::DictValue& subdict) {
   internal::IndividualSettings* by_id = AccessById(extension_id);
-  if (by_id->Parse(subdict, internal::IndividualSettings::SCOPE_INDIVIDUAL))
+  if (by_id->Parse(subdict, internal::IndividualSettings::SCOPE_INDIVIDUAL)) {
     return true;
+  }
 
   settings_by_id_.erase(extension_id);
   InstallStageTrackerFactory::GetForBrowserContext(profile_)->ReportFailure(
@@ -935,8 +982,9 @@ internal::IndividualSettings* ExtensionManagement::GetSettingsForId(
   }
 
   auto iter_id = settings_by_id_.find(extension_id);
-  if (iter_id == settings_by_id_.end())
+  if (iter_id == settings_by_id_.end()) {
     return nullptr;
+  }
 
   return iter_id->second.get();
 }
@@ -958,8 +1006,9 @@ void ExtensionManagement::LoadDeferredExtensionSetting(
       continue;
     }
     const base::DictValue* subdict = iter.second.GetIfDict();
-    if (!subdict)
+    if (!subdict) {
       continue;
+    }
 
     auto extension_ids = base::SplitStringPiece(
         iter.first, ",", base::TRIM_WHITESPACE, base::SPLIT_WANT_NONEMPTY);
@@ -977,15 +1026,17 @@ const base::Value* ExtensionManagement::LoadPreference(
     const char* pref_name,
     bool force_managed,
     base::Value::Type expected_type) const {
-  if (!pref_service_)
+  if (!pref_service_) {
     return nullptr;
+  }
   const PrefService::Preference* pref =
       pref_service_->FindPreference(pref_name);
   if (pref && !pref->IsDefaultValue() &&
       (!force_managed || pref->IsManaged())) {
     const base::Value* value = pref->GetValue();
-    if (value && value->type() == expected_type)
+    if (value && value->type() == expected_type) {
       return value;
+    }
   }
   return nullptr;
 }
@@ -1016,8 +1067,9 @@ void ExtensionManagement::NotifyExtensionManagementPrefChanged() {
       InstallStageTracker::InstallCreationStage::NOTIFIED_FROM_MANAGEMENT,
       InstallStageTracker::InstallCreationStage::
           NOTIFIED_FROM_MANAGEMENT_NOT_FORCED);
-  for (auto& observer : observer_list_)
+  for (auto& observer : observer_list_) {
     observer.OnExtensionManagementSettingsChanged();
+  }
 }
 
 void ExtensionManagement::ReportExtensionManagementInstallCreationStage(
@@ -1055,8 +1107,9 @@ base::DictValue ExtensionManagement::GetInstallListByMode(
 
 void ExtensionManagement::UpdateForcedExtensions(
     const base::DictValue* extension_dict) {
-  if (!extension_dict)
+  if (!extension_dict) {
     return;
+  }
 
   InstallStageTracker* install_stage_tracker =
       InstallStageTrackerFactory::GetForBrowserContext(profile_);
