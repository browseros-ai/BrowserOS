diff --git a/chrome/browser/extensions/external_provider_manager.cc b/chrome/browser/extensions/external_provider_manager.cc
index 80ac525552c03f173ae13812bce4dc57222dec53..104131b75e62952b1ca0434d943990495455f5b3 100644
--- a/chrome/browser/extensions/external_provider_manager.cc
+++ b/chrome/browser/extensions/external_provider_manager.cc
@@ -291,6 +291,7 @@ bool ExternalProviderManager::OnExternalExtensionFileFound(
   installer->set_expected_version(info.version,
                                   true /* fail_install_if_unexpected */);
   installer->set_install_immediately(info.install_immediately);
+  installer->set_external_install_priority(info.install_priority);
   installer->set_creation_flags(info.creation_flags);
 
   CRXFileInfo file_info(
