diff --git a/chrome/common/chrome_paths.cc b/chrome/common/chrome_paths.cc
index 9c4d73b730baa..1722022daf3d5 100644
--- a/chrome/common/chrome_paths.cc
+++ b/chrome/common/chrome_paths.cc
@@ -512,6 +512,19 @@ bool PathProvider(int key, base::FilePath* result) {
       create_dir = true;
       break;
 
+    case chrome::DIR_BROWSEROS_BUNDLED_EXTENSIONS:
+#if BUILDFLAG(IS_MAC)
+      cur = base::apple::FrameworkBundlePath();
+      cur = cur.Append(FILE_PATH_LITERAL("Resources"))
+                .Append(FILE_PATH_LITERAL("browseros_extensions"));
+#else
+      if (!base::PathService::Get(base::DIR_MODULE, &cur)) {
+        return false;
+      }
+      cur = cur.Append(FILE_PATH_LITERAL("browseros_extensions"));
+#endif
+      break;
+
     default:
       return false;
   }
