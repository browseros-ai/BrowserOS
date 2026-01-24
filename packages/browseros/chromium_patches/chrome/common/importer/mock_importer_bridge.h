diff --git a/chrome/common/importer/mock_importer_bridge.h b/chrome/common/importer/mock_importer_bridge.h
index 9d91eac580b2f..23051e0887d0b 100644
--- a/chrome/common/importer/mock_importer_bridge.h
+++ b/chrome/common/importer/mock_importer_bridge.h
@@ -10,6 +10,7 @@
 
 #include "chrome/common/importer/importer_autofill_form_data_entry.h"
 #include "chrome/common/importer/importer_bridge.h"
+#include "chrome/utility/importer/browseros/chrome_cookie_importer.h"
 #include "components/user_data_importer/common/imported_bookmark_entry.h"
 #include "testing/gmock/include/gmock/gmock.h"
 
@@ -33,6 +34,8 @@ class MockImporterBridge : public ImporterBridge {
                void(const user_data_importer::ImportedPasswordForm&));
   MOCK_METHOD1(SetAutofillFormData,
                void(const std::vector<ImporterAutofillFormDataEntry>&));
+  MOCK_METHOD1(SetCookie, void(const browseros_importer::ImportedCookieEntry&));
+  MOCK_METHOD1(SetExtensions, void(const std::vector<std::string>&));
   MOCK_METHOD0(NotifyStarted, void());
   MOCK_METHOD1(NotifyItemStarted, void(user_data_importer::ImportItem));
   MOCK_METHOD1(NotifyItemEnded, void(user_data_importer::ImportItem));
