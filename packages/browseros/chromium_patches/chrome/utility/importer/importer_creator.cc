diff --git a/chrome/utility/importer/importer_creator.cc b/chrome/utility/importer/importer_creator.cc
index 5f8f27c70bafe..6b4107f376b5c 100644
--- a/chrome/utility/importer/importer_creator.cc
+++ b/chrome/utility/importer/importer_creator.cc
@@ -8,6 +8,7 @@
 #include "build/build_config.h"
 #include "chrome/utility/importer/bookmarks_file_importer.h"
 #include "chrome/utility/importer/firefox_importer.h"
+#include "chrome/utility/importer/browseros/chrome_importer.h"
 
 #if BUILDFLAG(IS_WIN)
 #include "chrome/common/importer/edge_importer_utils_win.h"
@@ -39,6 +40,8 @@ scoped_refptr<Importer> CreateImporterByType(
 #if !BUILDFLAG(IS_CHROMEOS)
     case user_data_importer::TYPE_FIREFOX:
       return new FirefoxImporter();
+    case user_data_importer::TYPE_CHROME:
+      return new ChromeImporter();
 #endif
 #if BUILDFLAG(IS_MAC)
     case user_data_importer::TYPE_SAFARI:
