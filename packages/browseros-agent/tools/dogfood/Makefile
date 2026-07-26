BINARY := browseros-dogfood
SOURCES := $(shell find . -name '*.go') go.mod go.sum
PREFIX ?= $(HOME)/bin

all: build

build: $(BINARY)

$(BINARY): $(SOURCES)
	@echo "[build] Compiling $(BINARY)..."
	@go build -o $(BINARY) .

.PHONY: all build install test clean

install: $(BINARY)
	@mkdir -p $(PREFIX)
	@cp $(BINARY) $(PREFIX)/$(BINARY)
	@codesign --force --sign - $(PREFIX)/$(BINARY) >/dev/null 2>&1 || true
	@echo "Installed $(PREFIX)/$(BINARY)"

test:
	@go test ./...

clean:
	@rm -f $(BINARY)
