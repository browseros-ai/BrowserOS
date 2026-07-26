BINARY := bpatch
CARGO ?= cargo
TARGET_DIR ?= target

GOBIN := $(shell go env GOBIN 2>/dev/null)
GOPATH := $(shell go env GOPATH 2>/dev/null)
ifeq ($(GOBIN),)
ifneq ($(GOPATH),)
GOBIN := $(GOPATH)/bin
endif
endif
ifeq ($(GOBIN),)
GOBIN := $(HOME)/.cargo/bin
endif
PREFIX ?= $(GOBIN)

.PHONY: build install uninstall clean test fmt

build:
	$(CARGO) build --release --locked --target-dir "$(TARGET_DIR)"

install: build
	mkdir -p "$(PREFIX)"
	install -m 0755 "$(TARGET_DIR)/release/$(BINARY)" "$(PREFIX)/$(BINARY)"
ifeq ($(shell uname -s),Darwin)
	codesign --force --sign - "$(PREFIX)/$(BINARY)"
endif
	@echo "Installed $(BINARY) to $(PREFIX)/$(BINARY)"

uninstall:
	rm -f "$(PREFIX)/$(BINARY)"
	@echo "Removed $(PREFIX)/$(BINARY)"

test:
	$(CARGO) test --locked

fmt:
	$(CARGO) fmt --all

clean:
	$(CARGO) clean --target-dir "$(TARGET_DIR)"
