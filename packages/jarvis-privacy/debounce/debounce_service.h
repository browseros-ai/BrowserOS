// Copyright (c) 2021 The Brave Authors. All rights reserved.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this file,
// You can obtain one at https://mozilla.org/MPL/2.0/.

#ifndef BRAVE_COMPONENTS_DEBOUNCE_CORE_BROWSER_DEBOUNCE_SERVICE_H_
#define BRAVE_COMPONENTS_DEBOUNCE_CORE_BROWSER_DEBOUNCE_SERVICE_H_

#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "components/keyed_service/core/keyed_service.h"
#include "components/prefs/pref_registry_simple.h"
#include "components/prefs/pref_service.h"

class GURL;

namespace debounce {

class DebounceComponentInstaller;

class DebounceService : public KeyedService {
 public:
  explicit DebounceService(DebounceComponentInstaller* component_installer,
                           PrefService* prefs);
  DebounceService(const DebounceService&) = delete;
  DebounceService& operator=(const DebounceService&) = delete;
  ~DebounceService() override;
  bool Debounce(const GURL& original_url, GURL* final_url) const;
  static void RegisterProfilePrefs(PrefRegistrySimple* registry);
  bool IsEnabled();
  void SetIsEnabled(const bool isEnabled);

 private:
  const raw_ptr<DebounceComponentInstaller> component_installer_;
  raw_ptr<PrefService> prefs_ = nullptr;
  base::WeakPtrFactory<DebounceService> weak_factory_{this};
};

}  // namespace debounce

#endif  // BRAVE_COMPONENTS_DEBOUNCE_CORE_BROWSER_DEBOUNCE_SERVICE_H_
