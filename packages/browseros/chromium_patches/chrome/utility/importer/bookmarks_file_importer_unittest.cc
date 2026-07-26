diff --git a/chrome/utility/importer/bookmarks_file_importer_unittest.cc b/chrome/utility/importer/bookmarks_file_importer_unittest.cc
index 253407be1260d..d18b0fc681338 100644
--- a/chrome/utility/importer/bookmarks_file_importer_unittest.cc
+++ b/chrome/utility/importer/bookmarks_file_importer_unittest.cc
@@ -15,6 +15,7 @@
 #include "base/time/time.h"
 #include "chrome/common/importer/importer_autofill_form_data_entry.h"
 #include "chrome/common/importer/importer_bridge.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
 #include "components/user_data_importer/common/imported_bookmark_entry.h"
 #include "components/user_data_importer/common/importer_data_types.h"
 #include "testing/gmock/include/gmock/gmock.h"
@@ -82,6 +83,14 @@ class MockImporterBridge : public ImporterBridge {
               SetAutofillFormData,
               (const std::vector<ImporterAutofillFormDataEntry>&),
               (override));
+  MOCK_METHOD(void,
+              SetCookie,
+              (const browseros_importer::ImportedCookieEntry&),
+              (override));
+  MOCK_METHOD(void,
+              SetExtensions,
+              (const std::vector<std::string>&),
+              (override));
 
  protected:
   ~MockImporterBridge() override = default;
