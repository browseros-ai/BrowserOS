BINARY := browseros-cli
SOURCES := $(shell find . -name '*.go')
VERSION ?= dev

$(BINARY): $(SOURCES)
	go build -ldflags "-X main.version=$(VERSION)" -o $(BINARY) .

.PHONY: install clean vet test

install:
	go install -ldflags "-X main.version=$(VERSION)" .

clean:
	rm -f $(BINARY)

vet:
	go vet ./...

test:
	go test -tags integration -v -timeout 120s ./...

release-dry:
	goreleaser release --snapshot --clean

release:
	goreleaser release --clean
