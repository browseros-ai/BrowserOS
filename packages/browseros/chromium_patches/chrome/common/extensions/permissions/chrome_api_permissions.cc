diff --git a/chrome/common/extensions/permissions/chrome_api_permissions.cc b/chrome/common/extensions/permissions/chrome_api_permissions.cc
index be5905cc05398..fade776a03d9b 100644
--- a/chrome/common/extensions/permissions/chrome_api_permissions.cc
+++ b/chrome/common/extensions/permissions/chrome_api_permissions.cc
@@ -67,6 +67,7 @@ constexpr APIPermissionInfo::InitInfo permissions_to_register[] = {
     {APIPermissionID::kBookmark, "bookmarks"},
     {APIPermissionID::kBrailleDisplayPrivate, "brailleDisplayPrivate",
      APIPermissionInfo::kFlagCannotBeOptional},
+    {APIPermissionID::kBrowserOS, "browserOS"},
     {APIPermissionID::kBrowsingData, "browsingData",
      APIPermissionInfo::kFlagDoesNotRequireManagedSessionFullLoginWarning},
     {APIPermissionID::kCertificateProvider, "certificateProvider",
