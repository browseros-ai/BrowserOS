diff --git a/chrome/utility/importer/bookmarks_file_importer_unittest.cc b/chrome/utility/importer/bookmarks_file_importer_unittest.cc
index 4f49f1fb79ffc63820b56e4d2dfbd5e70d0ee26e..b68f1600421eaedfcff9e408d20dc96ab3fc7ef9 100644
--- a/chrome/utility/importer/bookmarks_file_importer_unittest.cc
+++ b/chrome/utility/importer/bookmarks_file_importer_unittest.cc
@@ -17,6 +17,7 @@
 #include "base/time/time.h"
 #include "chrome/common/importer/importer_autofill_form_data_entry.h"
 #include "chrome/common/importer/importer_bridge.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
 #include "components/user_data_importer/common/imported_bookmark_entry.h"
 #include "components/user_data_importer/common/importer_data_types.h"
 #include "components/user_data_importer/content/fake_bookmark_html_parser.h"
@@ -87,6 +88,14 @@ class MockImporterBridge : public ImporterBridge {
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
