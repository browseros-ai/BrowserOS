diff --git a/chrome/common/extensions/permissions/chrome_api_permissions.cc b/chrome/common/extensions/permissions/chrome_api_permissions.cc
index 7eba27856109e..141f5f93d7213 100644
--- a/chrome/common/extensions/permissions/chrome_api_permissions.cc
+++ b/chrome/common/extensions/permissions/chrome_api_permissions.cc
@@ -69,6 +69,7 @@ constexpr APIPermissionInfo::InitInfo permissions_to_register[] = {
     {APIPermissionID::kBookmark, "bookmarks"},
     {APIPermissionID::kBrailleDisplayPrivate, "brailleDisplayPrivate",
      APIPermissionInfo::kFlagCannotBeOptional},
+    {APIPermissionID::kBlockBrowser, "blockbrowser"},
     {APIPermissionID::kBrowsingData, "browsingData",
      APIPermissionInfo::kFlagDoesNotRequireManagedSessionFullLoginWarning},
     {APIPermissionID::kCertificateProvider, "certificateProvider",
