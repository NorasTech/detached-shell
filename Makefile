# NDS Makefile
# Makefile for quick commands

.PHONY: help build release test install clean format check watch quick doc

# Default target
help:
	@echo "NDS Development Makefile"
	@echo ""
	@echo "Usage: make <target>"
	@echo ""
	@echo "Targets:"
	@echo "  build    - Debug build"
	@echo "  release  - Release build (optimized)"
	@echo "  test     - Run tests"
	@echo "  install  - Release build and install to system"
	@echo "  clean    - Clean up"
	@echo "  format   - Code formatting"
	@echo "  check    - Lint and format check"
	@echo "  watch    - Watch file changes"
	@echo "  quick    - Quick build & install"
	@echo "  doc      - Generate documentation"

# Development commands
build:
	@./scripts/dev.sh build

release:
	@./scripts/dev.sh release

test:
	@./scripts/dev.sh test

install:
	@./scripts/install.sh

clean:
	@./scripts/dev.sh clean

format:
	@./scripts/dev.sh format

check:
	@./scripts/dev.sh check

watch:
	@./scripts/dev.sh watch

quick:
	@./scripts/dev.sh quick

doc:
	@./scripts/dev.sh doc

# Shortcuts
b: build
r: release
t: test
i: install
c: clean
f: format
q: quick

# Combinations
all: format check test release install
	@echo "✅ All operations completed!"

ci: format check test
	@echo "✅ CI checks successful!"