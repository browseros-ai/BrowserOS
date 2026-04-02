BINARY := browseros-cli
SOURCES := $(shell find . -name '*.go')
VERSION ?= dev
POSTHOG_API_KEY ?=
DIST := dist
LDFLAGS := -X main.version=$(VERSION) -X browseros-cli/analytics.posthogAPIKey=$(POSTHOG_API_KEY)
HOST_OS := $(shell go env GOOS)
HOST_ARCH := $(shell go env GOARCH)
HOST_EXT := $(if $(filter windows,$(HOST_OS)),.exe,)
HOST_BINARY = $(DIST)/$(BINARY)_$(HOST_OS)_$(HOST_ARCH)$(HOST_EXT)

$(BINARY): $(SOURCES)
	go build -ldflags "$(LDFLAGS)" -o $(BINARY) .

PLATFORMS := darwin/amd64 darwin/arm64 linux/amd64 linux/arm64 windows/amd64 windows/arm64

.PHONY: install clean vet test release

install:
	go install -ldflags "$(LDFLAGS)" .

clean:
	rm -f $(BINARY)
	rm -rf $(DIST)

vet:
	go vet ./...

test:
	go test -tags integration -v -timeout 120s ./...

release:
	@if [ "$(VERSION)" = "dev" ]; then echo "Error: VERSION required (e.g. make release VERSION=0.1.0)" >&2; exit 1; fi
	@rm -rf $(DIST) && mkdir -p $(DIST)
	@for pair in $(PLATFORMS); do \
		OS=$${pair%/*}; \
		ARCH=$${pair#*/}; \
		EXT=""; \
		if [ "$$OS" = "windows" ]; then EXT=".exe"; fi; \
		echo "Building $$OS/$$ARCH..."; \
		GOOS=$$OS GOARCH=$$ARCH CGO_ENABLED=0 go build -trimpath \
			-ldflags "-s -w $(LDFLAGS)" \
			-o "$(DIST)/$(BINARY)$$EXT" .; \
		ARCHIVE="$(BINARY)_$(VERSION)_$${OS}_$${ARCH}"; \
		if [ "$$OS" = "windows" ]; then \
			(cd $(DIST) && zip "$${ARCHIVE}.zip" "$(BINARY)$$EXT"); \
		else \
			(cd $(DIST) && tar czf "$${ARCHIVE}.tar.gz" "$(BINARY)"); \
		fi; \
		mv "$(DIST)/$(BINARY)$$EXT" "$(DIST)/$(BINARY)_$${OS}_$${ARCH}$$EXT"; \
	done
	@ACTUAL_VERSION=$$($(HOST_BINARY) --version | awk '{print $$3}'); \
		if [ "$$ACTUAL_VERSION" != "$(VERSION)" ]; then \
			echo "Error: expected $(HOST_BINARY) to report version $(VERSION), got $$ACTUAL_VERSION" >&2; \
			exit 1; \
		fi
	@cd $(DIST) && (command -v sha256sum >/dev/null 2>&1 && sha256sum *.tar.gz *.zip || shasum -a 256 *.tar.gz *.zip) > checksums.txt
	@echo "=== Built artifacts ==="
	@ls -lh $(DIST)

.PHONY: npm-version npm-publish

npm-version:
	@if [ "$(VERSION)" = "dev" ]; then echo "Error: VERSION required" >&2; exit 1; fi
	@node -e "const p=require('./npm/package.json');p.version='$(VERSION)';require('fs').writeFileSync('./npm/package.json',JSON.stringify(p,null,2)+'\n')"
	@echo "npm/package.json version set to $(VERSION)"

npm-publish: npm-version
	cd npm && npm publish
