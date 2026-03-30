BINARY := bdev
PREFIX ?= /usr/local/bin
VERSION ?= dev

.PHONY: build install clean test fmt

build:
	go build -ldflags "-X github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/cmd.Version=$(VERSION)" -o $(BINARY) .

install: build
	mkdir -p $(PREFIX)
	cp $(BINARY) $(PREFIX)/$(BINARY)
ifneq ($(shell uname -s),Darwin)
	@echo "Skipping codesign on non-macOS host"
else
	codesign --force --sign - $(PREFIX)/$(BINARY)
endif
	@echo "Installed $(BINARY) to $(PREFIX)/$(BINARY)"

test:
	go test ./...

fmt:
	gofmt -w $$(find . -name '*.go' -not -path './vendor/*')

clean:
	rm -f $(BINARY)
