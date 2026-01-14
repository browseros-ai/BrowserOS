diff --git a/chrome/utility/importer/external_process_importer_bridge.cc b/chrome/utility/importer/external_process_importer_bridge.cc
index 67092331c3801..df9cfc2576fd8 100644
--- a/chrome/utility/importer/external_process_importer_bridge.cc
+++ b/chrome/utility/importer/external_process_importer_bridge.cc
@@ -15,8 +15,11 @@
 #include "base/task/task_runner.h"
 #include "build/build_config.h"
 #include "chrome/common/importer/importer_autofill_form_data_entry.h"
+#include "chrome/common/importer/profile_import.mojom.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
 #include "components/user_data_importer/common/imported_bookmark_entry.h"
 #include "components/user_data_importer/common/importer_data_types.h"
+#include "net/cookies/cookie_constants.h"
 
 namespace {
 
@@ -113,6 +116,79 @@ void ExternalProcessImporterBridge::SetPasswordForm(
   observer_->OnPasswordFormImportReady(form);
 }
 
+void ExternalProcessImporterBridge::SetCookie(
+    const browseros_importer::ImportedCookieEntry& cookie) {
+  auto mojo_cookie = chrome::mojom::ImportedCookieEntry::New();
+  mojo_cookie->host_key = cookie.host_key;
+  mojo_cookie->name = cookie.name;
+  mojo_cookie->value = cookie.value;
+  mojo_cookie->path = cookie.path;
+  mojo_cookie->expires_utc =
+      cookie.expires_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  mojo_cookie->creation_utc =
+      cookie.creation_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  mojo_cookie->last_access_utc =
+      cookie.last_access_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  mojo_cookie->last_update_utc =
+      cookie.last_update_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  mojo_cookie->is_secure = cookie.is_secure;
+  mojo_cookie->is_httponly = cookie.is_httponly;
+
+  switch (cookie.same_site) {
+    case net::CookieSameSite::UNSPECIFIED:
+      mojo_cookie->same_site =
+          chrome::mojom::ImportedCookieEntry::SameSite::kUnspecified;
+      break;
+    case net::CookieSameSite::NO_RESTRICTION:
+      mojo_cookie->same_site =
+          chrome::mojom::ImportedCookieEntry::SameSite::kNoRestriction;
+      break;
+    case net::CookieSameSite::LAX_MODE:
+      mojo_cookie->same_site =
+          chrome::mojom::ImportedCookieEntry::SameSite::kLaxMode;
+      break;
+    case net::CookieSameSite::STRICT_MODE:
+      mojo_cookie->same_site =
+          chrome::mojom::ImportedCookieEntry::SameSite::kStrictMode;
+      break;
+  }
+
+  switch (cookie.priority) {
+    case net::COOKIE_PRIORITY_LOW:
+      mojo_cookie->priority =
+          chrome::mojom::ImportedCookieEntry::Priority::kLow;
+      break;
+    case net::COOKIE_PRIORITY_MEDIUM:
+      mojo_cookie->priority =
+          chrome::mojom::ImportedCookieEntry::Priority::kMedium;
+      break;
+    case net::COOKIE_PRIORITY_HIGH:
+      mojo_cookie->priority =
+          chrome::mojom::ImportedCookieEntry::Priority::kHigh;
+      break;
+  }
+
+  switch (cookie.source_scheme) {
+    case net::CookieSourceScheme::kUnset:
+      mojo_cookie->source_scheme =
+          chrome::mojom::ImportedCookieEntry::SourceScheme::kUnset;
+      break;
+    case net::CookieSourceScheme::kNonSecure:
+      mojo_cookie->source_scheme =
+          chrome::mojom::ImportedCookieEntry::SourceScheme::kNonSecure;
+      break;
+    case net::CookieSourceScheme::kSecure:
+      mojo_cookie->source_scheme =
+          chrome::mojom::ImportedCookieEntry::SourceScheme::kSecure;
+      break;
+  }
+
+  mojo_cookie->source_port = cookie.source_port;
+  mojo_cookie->is_persistent = cookie.is_persistent;
+
+  observer_->OnCookieImportReady(std::move(mojo_cookie));
+}
+
 void ExternalProcessImporterBridge::SetAutofillFormData(
     const std::vector<ImporterAutofillFormDataEntry>& entries) {
   observer_->OnAutofillFormDataImportStart(entries.size());
@@ -135,6 +211,13 @@ void ExternalProcessImporterBridge::SetAutofillFormData(
   DCHECK_EQ(0, autofill_form_data_entries_left);
 }
 
+void ExternalProcessImporterBridge::SetExtensions(
+    const std::vector<std::string>& extension_ids) {
+  // Since extension installations need to be handled by the browser process,
+  // we'll just pass this information through
+  observer_->OnExtensionsImportReady(extension_ids);
+}
+
 void ExternalProcessImporterBridge::NotifyStarted() {
   observer_->OnImportStart();
 }
