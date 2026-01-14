diff --git a/chrome/common/importer/profile_import_process_param_traits.h b/chrome/common/importer/profile_import_process_param_traits.h
index 61ec7a490c05e..2f62b6deb0727 100644
--- a/chrome/common/importer/profile_import_process_param_traits.h
+++ b/chrome/common/importer/profile_import_process_param_traits.h
@@ -8,11 +8,14 @@
 #include <string>
 
 #include "base/notreached.h"
+#include "base/time/time.h"
 #include "chrome/common/importer/profile_import.mojom.h"
 #include "chrome/common/importer/profile_import_process_param_traits_macros.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
 #include "components/user_data_importer/common/importer_data_types.h"
 #include "mojo/public/cpp/bindings/enum_traits.h"
 #include "mojo/public/cpp/bindings/struct_traits.h"
+#include "net/cookies/cookie_constants.h"
 
 namespace mojo {
 
@@ -96,6 +99,191 @@ struct StructTraits<chrome::mojom::ImportedPasswordFormDataView,
                    user_data_importer::ImportedPasswordForm* out);
 };
 
+// Enum traits for cookie SameSite
+template <>
+struct EnumTraits<chrome::mojom::ImportedCookieEntry::SameSite,
+                  net::CookieSameSite> {
+  static chrome::mojom::ImportedCookieEntry::SameSite ToMojom(
+      net::CookieSameSite input) {
+    switch (input) {
+      case net::CookieSameSite::UNSPECIFIED:
+        return chrome::mojom::ImportedCookieEntry::SameSite::kUnspecified;
+      case net::CookieSameSite::NO_RESTRICTION:
+        return chrome::mojom::ImportedCookieEntry::SameSite::kNoRestriction;
+      case net::CookieSameSite::LAX_MODE:
+        return chrome::mojom::ImportedCookieEntry::SameSite::kLaxMode;
+      case net::CookieSameSite::STRICT_MODE:
+        return chrome::mojom::ImportedCookieEntry::SameSite::kStrictMode;
+    }
+    NOTREACHED();
+  }
+
+  static bool FromMojom(chrome::mojom::ImportedCookieEntry::SameSite input,
+                        net::CookieSameSite* out) {
+    switch (input) {
+      case chrome::mojom::ImportedCookieEntry::SameSite::kUnspecified:
+        *out = net::CookieSameSite::UNSPECIFIED;
+        return true;
+      case chrome::mojom::ImportedCookieEntry::SameSite::kNoRestriction:
+        *out = net::CookieSameSite::NO_RESTRICTION;
+        return true;
+      case chrome::mojom::ImportedCookieEntry::SameSite::kLaxMode:
+        *out = net::CookieSameSite::LAX_MODE;
+        return true;
+      case chrome::mojom::ImportedCookieEntry::SameSite::kStrictMode:
+        *out = net::CookieSameSite::STRICT_MODE;
+        return true;
+    }
+    NOTREACHED();
+  }
+};
+
+// Enum traits for cookie Priority
+template <>
+struct EnumTraits<chrome::mojom::ImportedCookieEntry::Priority,
+                  net::CookiePriority> {
+  static chrome::mojom::ImportedCookieEntry::Priority ToMojom(
+      net::CookiePriority input) {
+    switch (input) {
+      case net::COOKIE_PRIORITY_LOW:
+        return chrome::mojom::ImportedCookieEntry::Priority::kLow;
+      case net::COOKIE_PRIORITY_MEDIUM:
+        return chrome::mojom::ImportedCookieEntry::Priority::kMedium;
+      case net::COOKIE_PRIORITY_HIGH:
+        return chrome::mojom::ImportedCookieEntry::Priority::kHigh;
+    }
+    NOTREACHED();
+  }
+
+  static bool FromMojom(chrome::mojom::ImportedCookieEntry::Priority input,
+                        net::CookiePriority* out) {
+    switch (input) {
+      case chrome::mojom::ImportedCookieEntry::Priority::kLow:
+        *out = net::COOKIE_PRIORITY_LOW;
+        return true;
+      case chrome::mojom::ImportedCookieEntry::Priority::kMedium:
+        *out = net::COOKIE_PRIORITY_MEDIUM;
+        return true;
+      case chrome::mojom::ImportedCookieEntry::Priority::kHigh:
+        *out = net::COOKIE_PRIORITY_HIGH;
+        return true;
+    }
+    NOTREACHED();
+  }
+};
+
+// Enum traits for cookie SourceScheme
+template <>
+struct EnumTraits<chrome::mojom::ImportedCookieEntry::SourceScheme,
+                  net::CookieSourceScheme> {
+  static chrome::mojom::ImportedCookieEntry::SourceScheme ToMojom(
+      net::CookieSourceScheme input) {
+    switch (input) {
+      case net::CookieSourceScheme::kUnset:
+        return chrome::mojom::ImportedCookieEntry::SourceScheme::kUnset;
+      case net::CookieSourceScheme::kNonSecure:
+        return chrome::mojom::ImportedCookieEntry::SourceScheme::kNonSecure;
+      case net::CookieSourceScheme::kSecure:
+        return chrome::mojom::ImportedCookieEntry::SourceScheme::kSecure;
+    }
+    NOTREACHED();
+  }
+
+  static bool FromMojom(chrome::mojom::ImportedCookieEntry::SourceScheme input,
+                        net::CookieSourceScheme* out) {
+    switch (input) {
+      case chrome::mojom::ImportedCookieEntry::SourceScheme::kUnset:
+        *out = net::CookieSourceScheme::kUnset;
+        return true;
+      case chrome::mojom::ImportedCookieEntry::SourceScheme::kNonSecure:
+        *out = net::CookieSourceScheme::kNonSecure;
+        return true;
+      case chrome::mojom::ImportedCookieEntry::SourceScheme::kSecure:
+        *out = net::CookieSourceScheme::kSecure;
+        return true;
+    }
+    NOTREACHED();
+  }
+};
+
+// Struct traits for ImportedCookieEntry
+template <>
+struct StructTraits<chrome::mojom::ImportedCookieEntryDataView,
+                    browseros_importer::ImportedCookieEntry> {
+  static const std::string& host_key(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.host_key;
+  }
+
+  static const std::string& name(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.name;
+  }
+
+  static const std::string& value(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.value;
+  }
+
+  static const std::string& path(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.path;
+  }
+
+  static int64_t expires_utc(const browseros_importer::ImportedCookieEntry& r) {
+    return r.expires_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  }
+
+  static int64_t creation_utc(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.creation_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  }
+
+  static int64_t last_access_utc(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.last_access_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  }
+
+  static int64_t last_update_utc(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.last_update_utc.ToDeltaSinceWindowsEpoch().InMicroseconds();
+  }
+
+  static bool is_secure(const browseros_importer::ImportedCookieEntry& r) {
+    return r.is_secure;
+  }
+
+  static bool is_httponly(const browseros_importer::ImportedCookieEntry& r) {
+    return r.is_httponly;
+  }
+
+  static net::CookieSameSite same_site(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.same_site;
+  }
+
+  static net::CookiePriority priority(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.priority;
+  }
+
+  static net::CookieSourceScheme source_scheme(
+      const browseros_importer::ImportedCookieEntry& r) {
+    return r.source_scheme;
+  }
+
+  static int32_t source_port(const browseros_importer::ImportedCookieEntry& r) {
+    return r.source_port;
+  }
+
+  static bool is_persistent(const browseros_importer::ImportedCookieEntry& r) {
+    return r.is_persistent;
+  }
+
+  static bool Read(chrome::mojom::ImportedCookieEntryDataView data,
+                   browseros_importer::ImportedCookieEntry* out);
+};
+
 }  // namespace mojo
 
 #endif  // CHROME_COMMON_IMPORTER_PROFILE_IMPORT_PROCESS_PARAM_TRAITS_H_
