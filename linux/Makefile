.PHONY: build run test lint install install-shortcut uninstall package clean

build:
	cargo build --release

run:
	cargo run

# Integration tests; set KITELAB_MYSQL / KITELAB_PG=host:port:user:password
test:
	cargo test

lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

install: build
	./scripts/install.sh

install-shortcut: build
	./scripts/install.sh --desktop-shortcut

uninstall:
	./scripts/uninstall.sh

package:
	./scripts/package.sh

clean:
	cargo clean
	rm -rf build
