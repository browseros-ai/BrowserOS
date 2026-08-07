diff --git a/chrome/common/chrome_constants.cc b/chrome/common/chrome_constants.cc
index 6e30ef11474c457acf1bca690c6a310afb11d751..a87732bff1ae008949f8d86f70e35c98ba268f55 100644
--- a/chrome/common/chrome_constants.cc
+++ b/chrome/common/chrome_constants.cc
@@ -5,6 +5,7 @@
 #include "chrome/common/chrome_constants.h"
 
 #include "build/build_config.h"
+#include "chrome/browser/browseros/buildflags.h"
 #include "chrome/common/chrome_version.h"
 
 #define FPL FILE_PATH_LITERAL
@@ -46,7 +47,12 @@ const base::FilePath::CharType kBrowserProcessExecutableName[] = FPL("chrome");
 const base::FilePath::CharType kHelperProcessExecutableName[] =
     FPL("sandboxed_process");
 #elif BUILDFLAG(IS_POSIX)
-const base::FilePath::CharType kBrowserProcessExecutableName[] = FPL("chrome");
+#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+const base::FilePath::CharType kBrowserProcessExecutableName[] =
+    FPL("browserclaw");
+#else
+const base::FilePath::CharType kBrowserProcessExecutableName[] = FPL("browseros");
+#endif
 // Helper processes end up with a name of "exe" due to execing via
 // /proc/self/exe.  See bug 22703.
 const base::FilePath::CharType kHelperProcessExecutableName[] = FPL("exe");
@@ -75,8 +81,15 @@ const base::FilePath::CharType kHelperProcessExecutablePath[] =
 const base::FilePath::CharType kBrowserProcessExecutablePath[] = FPL("chrome");
 const base::FilePath::CharType kHelperProcessExecutablePath[] = FPL("chrome");
 #elif BUILDFLAG(IS_POSIX)
-const base::FilePath::CharType kBrowserProcessExecutablePath[] = FPL("chrome");
-const base::FilePath::CharType kHelperProcessExecutablePath[] = FPL("chrome");
+#if BUILDFLAG(BROWSEROS_PRODUCT_BROWSERCLAW)
+const base::FilePath::CharType kBrowserProcessExecutablePath[] =
+    FPL("browserclaw");
+const base::FilePath::CharType kHelperProcessExecutablePath[] =
+    FPL("browserclaw");
+#else
+const base::FilePath::CharType kBrowserProcessExecutablePath[] = FPL("browseros");
+const base::FilePath::CharType kHelperProcessExecutablePath[] = FPL("browseros");
+#endif
 #endif  // OS_*
 
 #if BUILDFLAG(IS_MAC)
