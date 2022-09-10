build:
	cargo build --profile release-lto

install:
	cp ./target/release-lto/bfirs /usr/bin/
