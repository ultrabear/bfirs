# ultrabear/bfirs
A rust port of [ultrabear/bfi](https://github.com/ultrabear/bfi)  
This implementation is faster than bfi and served as a tool to better learn rust as a language. It uses the same algorithms from the go version with some tweaking to work in the context of rust.
# Installing
The MSRV (Minimum Supported Rust Version) of this project is currently 1.61, but this is subject to increase so using "latest" as an MSRV is more appropriate.  
If you are familiar with cargo you may build this project normally, the release profile has been reconfigured to fit the project.  
A Makefile is provided with simple `make` and `make install` commands for anyone who does not wish to use cargo directly, but rustc and cargo must be installed regardless.
# Differences from bfi
`bfirs` removes the automatic compression that `bfi` does, this means `+[]` will never halt in `bfirs`. `bfirs` also adds support for 16 and 32 bit execution modes. Additionally `bfirs` requires flag arguments to be passed, unlike `bfi` that takes argv as code by default
