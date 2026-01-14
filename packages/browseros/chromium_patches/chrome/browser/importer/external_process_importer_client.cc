diff --git a/chrome/browser/importer/external_process_importer_client.cc b/chrome/browser/importer/external_process_importer_client.cc
index 6ee7a959fde3e..e60d680b1a99b 100644
--- a/chrome/browser/importer/external_process_importer_client.cc
+++ b/chrome/browser/importer/external_process_importer_client.cc
@@ -14,10 +14,12 @@
 #include "chrome/common/importer/firefox_importer_utils.h"
 #include "chrome/common/importer/profile_import.mojom.h"
 #include "chrome/grit/generated_resources.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
 #include "components/strings/grit/components_strings.h"
 #include "components/user_data_importer/common/imported_bookmark_entry.h"
 #include "content/public/browser/child_process_host.h"
 #include "content/public/browser/service_process_host.h"
+#include "net/cookies/cookie_constants.h"
 #include "ui/base/l10n/l10n_util.h"
 
 ExternalProcessImporterClient::ExternalProcessImporterClient(
@@ -221,6 +223,72 @@ void ExternalProcessImporterClient::OnPasswordFormImportReady(
   bridge_->SetPasswordForm(form);
 }
 
+void ExternalProcessImporterClient::OnCookieImportReady(
+    chrome::mojom::ImportedCookieEntryPtr mojo_cookie) {
+  if (cancelled_)
+    return;
+
+  browseros_importer::ImportedCookieEntry cookie;
+  cookie.host_key = mojo_cookie->host_key;
+  cookie.name = mojo_cookie->name;
+  cookie.value = mojo_cookie->value;
+  cookie.path = mojo_cookie->path;
+  cookie.expires_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(mojo_cookie->expires_utc));
+  cookie.creation_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(mojo_cookie->creation_utc));
+  cookie.last_access_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(mojo_cookie->last_access_utc));
+  cookie.last_update_utc = base::Time::FromDeltaSinceWindowsEpoch(
+      base::Microseconds(mojo_cookie->last_update_utc));
+  cookie.is_secure = mojo_cookie->is_secure;
+  cookie.is_httponly = mojo_cookie->is_httponly;
+
+  switch (mojo_cookie->same_site) {
+    case chrome::mojom::ImportedCookieEntry::SameSite::kUnspecified:
+      cookie.same_site = net::CookieSameSite::UNSPECIFIED;
+      break;
+    case chrome::mojom::ImportedCookieEntry::SameSite::kNoRestriction:
+      cookie.same_site = net::CookieSameSite::NO_RESTRICTION;
+      break;
+    case chrome::mojom::ImportedCookieEntry::SameSite::kLaxMode:
+      cookie.same_site = net::CookieSameSite::LAX_MODE;
+      break;
+    case chrome::mojom::ImportedCookieEntry::SameSite::kStrictMode:
+      cookie.same_site = net::CookieSameSite::STRICT_MODE;
+      break;
+  }
+
+  switch (mojo_cookie->priority) {
+    case chrome::mojom::ImportedCookieEntry::Priority::kLow:
+      cookie.priority = net::COOKIE_PRIORITY_LOW;
+      break;
+    case chrome::mojom::ImportedCookieEntry::Priority::kMedium:
+      cookie.priority = net::COOKIE_PRIORITY_MEDIUM;
+      break;
+    case chrome::mojom::ImportedCookieEntry::Priority::kHigh:
+      cookie.priority = net::COOKIE_PRIORITY_HIGH;
+      break;
+  }
+
+  switch (mojo_cookie->source_scheme) {
+    case chrome::mojom::ImportedCookieEntry::SourceScheme::kUnset:
+      cookie.source_scheme = net::CookieSourceScheme::kUnset;
+      break;
+    case chrome::mojom::ImportedCookieEntry::SourceScheme::kNonSecure:
+      cookie.source_scheme = net::CookieSourceScheme::kNonSecure;
+      break;
+    case chrome::mojom::ImportedCookieEntry::SourceScheme::kSecure:
+      cookie.source_scheme = net::CookieSourceScheme::kSecure;
+      break;
+  }
+
+  cookie.source_port = mojo_cookie->source_port;
+  cookie.is_persistent = mojo_cookie->is_persistent;
+
+  bridge_->SetCookie(cookie);
+}
+
 void ExternalProcessImporterClient::OnKeywordsImportReady(
     const std::vector<user_data_importer::SearchEngineInfo>& search_engines,
     bool unique_on_host_and_path) {
@@ -251,6 +319,14 @@ void ExternalProcessImporterClient::OnAutofillFormDataImportGroup(
     bridge_->SetAutofillFormData(autofill_form_data_);
 }
 
+void ExternalProcessImporterClient::OnExtensionsImportReady(
+    const std::vector<std::string>& extension_ids) {
+  if (cancelled_)
+    return;
+
+  bridge_->SetExtensions(extension_ids);
+}
+
 ExternalProcessImporterClient::~ExternalProcessImporterClient() = default;
 
 void ExternalProcessImporterClient::Cleanup() {
