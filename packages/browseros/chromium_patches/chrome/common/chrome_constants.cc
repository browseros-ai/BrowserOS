diff --git a/chrome/common/chrome_constants.cc b/chrome/common/chrome_constants.cc
index 298138e77b9f6..d89ef96cc4d86 100644
--- a/chrome/common/chrome_constants.cc
+++ b/chrome/common/chrome_constants.cc
@@ -33,9 +33,9 @@ const char kChromeVersion[] = CHROME_VERSION_STRING;
 
 #if BUILDFLAG(IS_WIN)
 const base::FilePath::CharType kBrowserProcessExecutableName[] =
-    FPL("chrome.exe");
+    FPL("browseros.exe");
 const base::FilePath::CharType kHelperProcessExecutableName[] =
-    FPL("chrome.exe");
+    FPL("browseros.exe");
 #elif BUILDFLAG(IS_MAC)
 const base::FilePath::CharType kBrowserProcessExecutableName[] =
     FPL(PRODUCT_FULLNAME_STRING);
@@ -47,7 +47,7 @@ const base::FilePath::CharType kBrowserProcessExecutableName[] = FPL("chrome");
 const base::FilePath::CharType kHelperProcessExecutableName[] =
     FPL("sandboxed_process");
 #elif BUILDFLAG(IS_POSIX)
-const base::FilePath::CharType kBrowserProcessExecutableName[] = FPL("chrome");
+const base::FilePath::CharType kBrowserProcessExecutableName[] = FPL("browseros");
 // Helper processes end up with a name of "exe" due to execing via
 // /proc/self/exe.  See bug 22703.
 const base::FilePath::CharType kHelperProcessExecutableName[] = FPL("exe");
@@ -55,9 +55,9 @@ const base::FilePath::CharType kHelperProcessExecutableName[] = FPL("exe");
 
 #if BUILDFLAG(IS_WIN)
 const base::FilePath::CharType kBrowserProcessExecutablePath[] =
-    FPL("chrome.exe");
+    FPL("browseros.exe");
 const base::FilePath::CharType kHelperProcessExecutablePath[] =
-    FPL("chrome.exe");
+    FPL("browseros.exe");
 #elif BUILDFLAG(IS_MAC)
 const base::FilePath::CharType kBrowserProcessExecutablePath[] =
     FPL(PRODUCT_FULLNAME_STRING ".app/Contents/MacOS/" PRODUCT_FULLNAME_STRING);
@@ -76,8 +76,8 @@ const base::FilePath::CharType kHelperProcessExecutablePath[] =
 const base::FilePath::CharType kBrowserProcessExecutablePath[] = FPL("chrome");
 const base::FilePath::CharType kHelperProcessExecutablePath[] = FPL("chrome");
 #elif BUILDFLAG(IS_POSIX)
-const base::FilePath::CharType kBrowserProcessExecutablePath[] = FPL("chrome");
-const base::FilePath::CharType kHelperProcessExecutablePath[] = FPL("chrome");
+const base::FilePath::CharType kBrowserProcessExecutablePath[] = FPL("browseros");
+const base::FilePath::CharType kHelperProcessExecutablePath[] = FPL("browseros");
 #endif  // OS_*
 
 #if BUILDFLAG(IS_MAC)
@@ -89,8 +89,8 @@ const char kMacHelperSuffixAlerts[] = " (Alerts)";
 #endif  // BUILDFLAG(IS_MAC)
 
 #if BUILDFLAG(IS_WIN)
-const base::FilePath::CharType kBrowserResourcesDll[] = FPL("chrome.dll");
-const base::FilePath::CharType kElfDll[] = FPL("chrome_elf.dll");
+const base::FilePath::CharType kBrowserResourcesDll[] = FPL("browseros.dll");
+const base::FilePath::CharType kElfDll[] = FPL("blockbrowser_elf.dll");
 const base::FilePath::CharType kStatusTrayWindowClass[] =
     FPL("Chrome_StatusTrayWindow");
 #endif  // BUILDFLAG(IS_WIN)
