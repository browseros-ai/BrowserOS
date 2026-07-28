diff --git a/components/user_data_importer/common/importer_type.h b/components/user_data_importer/common/importer_type.h
index d4f7de8d9a49a..bfe49d790d5ab 100644
--- a/components/user_data_importer/common/importer_type.h
+++ b/components/user_data_importer/common/importer_type.h
@@ -28,6 +28,7 @@ enum ImporterType {
 #if BUILDFLAG(IS_WIN)
   TYPE_EDGE = 6,
 #endif
+  TYPE_CHROME = 7,
 };
 
 }  // namespace user_data_importer
