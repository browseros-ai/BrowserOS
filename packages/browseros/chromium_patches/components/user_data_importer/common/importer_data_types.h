diff --git a/components/user_data_importer/common/importer_data_types.h b/components/user_data_importer/common/importer_data_types.h
index 3cac91f8d5838..8d5937f653020 100644
--- a/components/user_data_importer/common/importer_data_types.h
+++ b/components/user_data_importer/common/importer_data_types.h
@@ -24,12 +24,13 @@ enum ImportItem {
   NONE = 0,
   HISTORY = 1 << 0,
   FAVORITES = 1 << 1,
-  COOKIES = 1 << 2,  // Not supported yet.
+  COOKIES = 1 << 2,
   PASSWORDS = 1 << 3,
   SEARCH_ENGINES = 1 << 4,
   HOME_PAGE = 1 << 5,
   AUTOFILL_FORM_DATA = 1 << 6,
-  ALL = (1 << 7) - 1  // All the bits should be 1, hence the -1.
+  EXTENSIONS = 1 << 7,
+  ALL = (1 << 8) - 1  // All the bits should be 1, hence the -1.
 };
 
 // Information about a profile needed by an importer to do import work.
@@ -111,6 +112,7 @@ enum VisitSource {
   VISIT_SOURCE_FIREFOX_IMPORTED = 1,
   VISIT_SOURCE_IE_IMPORTED = 2,
   VISIT_SOURCE_SAFARI_IMPORTED = 3,
+  VISIT_SOURCE_CHROME_IMPORTED = 4,
 };
 
 }  // namespace user_data_importer
