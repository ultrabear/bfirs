build:
	cargo build --profile release-lto

install: build
	cargo install --path .
