diff --git a/chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.cc b/chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.cc
new file mode 100644
index 0000000000000..b8c9d0e1f2a3b
--- /dev/null
+++ b/chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.cc
@@ -0,0 +1,72 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_service_factory.h"
+
+#include <memory>
+
+#include "chrome/browser/browseros/ghost_mode/ghost_mode_service.h"
+#include "chrome/browser/profiles/incognito_helpers.h"
+#include "chrome/browser/profiles/profile.h"
+#include "components/keyed_service/content/browser_context_dependency_manager.h"
+
+namespace browseros::ghost_mode {
+
+// static
+GhostModeService* GhostModeServiceFactory::GetForProfile(Profile* profile) {
+  return static_cast<GhostModeService*>(
+      GetInstance()->GetServiceForBrowserContext(profile, /*create=*/true));
+}
+
+// static
+GhostModeService* GhostModeServiceFactory::GetForProfileIfExists(
+    Profile* profile) {
+  return static_cast<GhostModeService*>(
+      GetInstance()->GetServiceForBrowserContext(profile, /*create=*/false));
+}
+
+// static
+GhostModeServiceFactory* GhostModeServiceFactory::GetInstance() {
+  static base::NoDestructor<GhostModeServiceFactory> instance;
+  return instance.get();
+}
+
+GhostModeServiceFactory::GhostModeServiceFactory()
+    : ProfileKeyedServiceFactory(
+          "GhostModeService",
+          ProfileSelections::Builder()
+              .WithRegular(ProfileSelection::kOriginalOnly)
+              .WithGuest(ProfileSelection::kNone)
+              .WithSystem(ProfileSelection::kNone)
+              .WithAshInternals(ProfileSelection::kNone)
+              .Build()) {
+  // Dependencies can be added here if needed
+  // DependsOn(OtherServiceFactory::GetInstance());
+}
+
+GhostModeServiceFactory::~GhostModeServiceFactory() = default;
+
+std::unique_ptr<KeyedService>
+GhostModeServiceFactory::BuildServiceInstanceForBrowserContext(
+    content::BrowserContext* context) const {
+  Profile* profile = Profile::FromBrowserContext(context);
+  
+  // Don't create for incognito
+  if (profile->IsOffTheRecord()) {
+    return nullptr;
+  }
+  
+  return std::make_unique<GhostModeService>(profile);
+}
+
+bool GhostModeServiceFactory::ServiceIsCreatedWithBrowserContext() const {
+  return true;
+}
+
+bool GhostModeServiceFactory::ServiceIsNULLWhileTesting() const {
+  return true;
+}
+
+content::BrowserContext* GhostModeServiceFactory::GetBrowserContextToUse(
+    content::BrowserContext* context) const {
+  return chrome::GetBrowserContextRedirectedInIncognito(context);
+}
+
+}  // namespace browseros::ghost_mode
