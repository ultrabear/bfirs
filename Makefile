build:
	cargo build --profile release-lto

install:
	cargo install --path . --profile release-lto
