# Mayara Build System
#
# Usage:
#   make          - Build release with docs (recommended)
#   make release  - Build release with docs
#   make debug    - Build debug with docs
#   make docs     - Generate rustdoc only
#   make run      - Build and run server
#   make clean    - Clean build artifacts

.PHONY: all release debug docs run clean test

# Default: build release with embedded docs
all: release

# Generate rustdoc for core and server
docs:
	@echo "Generating rustdoc..."
	cargo doc --no-deps -p mayara-core -p mayara-server
	@echo "Documentation generated at target/doc/"

# Build release binary with docs embedded
release: docs
	@echo "Building release..."
	cargo build --release -p mayara-server
	@echo ""
	@echo "Build complete: target/release/mayara-server"
	@echo "Rustdoc available at: http://localhost:6502/rustdoc/mayara_core/"

# Build debug binary with docs embedded
debug: docs
	@echo "Building debug..."
	cargo build -p mayara-server
	@echo ""
	@echo "Build complete: target/debug/mayara-server"
	@echo "Rustdoc available at: http://localhost:6502/rustdoc/mayara_core/"

# Build and run the server
run: release
	@echo "Starting server..."
	./target/release/mayara-server

# Run tests
test:
	cargo test -p mayara-core -p mayara-server

# Clean build artifacts
clean:
	cargo clean
