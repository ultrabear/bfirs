build:
	cargo build --profile release-lto --locked

install:
	cargo install --path . --profile release-lto --locked
