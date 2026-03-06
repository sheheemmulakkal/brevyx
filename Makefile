# Brevyx Makefile
#
# Targets:
#   make build     — debug build
#   make release   — optimised release build
#   make install   — release build + install via install.sh
#   make uninstall — remove via uninstall.sh
#   make test      — run all unit and integration tests
#   make lint      — cargo clippy -D warnings
#   make fmt       — auto-format with cargo fmt
#   make clean     — remove build artefacts

BIN        := brevyx
CARGO      := cargo
INSTALL_SH := ./install.sh

.PHONY: build release install uninstall test lint fmt clean

## build: compile in debug mode
build:
	$(CARGO) build

## release: compile optimised release binary
release:
	$(CARGO) build --release

## install: build release binary and install via install.sh
install: release
	$(INSTALL_SH)

## uninstall: remove binary, assets, and service via uninstall.sh
uninstall:
	./uninstall.sh

## test: run all unit and integration tests
test:
	$(CARGO) test

## lint: run clippy with zero-warnings policy
lint:
	$(CARGO) clippy -- -D warnings

## fmt: auto-format all source files
fmt:
	$(CARGO) fmt

## clean: remove target/ directory
clean:
	$(CARGO) clean

# ── Composite convenience targets ──────────────────────────────────────────────

## check: fmt check + clippy + tests (mirrors CI)
check:
	$(CARGO) fmt --check
	$(CARGO) clippy -- -D warnings
	$(CARGO) test

## ci: alias for check
ci: check
