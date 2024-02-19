# ultrabear/bfirs
A rust port of [ultrabear/bfi](https://github.com/ultrabear/bfi)  
This implementation is faster than bfi and served as a tool to better learn rust as a language. It uses the same algorithms from the go version with some tweaking to work in the context of rust.
# Installing
The MSRV (Minimum Supported Rust Version) of this project is currently 1.61, but this is subject to increase so using "latest" as an MSRV is more appropriate.  
If you are familiar with cargo you may build this project normally, the release profile has been reconfigured to fit the project.  
A Makefile is provided with simple `make` and `make install` commands for anyone who does not wish to use cargo directly, but rustc and cargo must be installed regardless.
# Differences from bfi
`bf` removes the automatic compression that `bfi` does, this means `+[]` will never halt in `bf`. `bf` also adds support for 16 and 32 bit execution modes. Additionally `bf` requires flag arguments to be passed, unlike `bfi` that takes argv as code by default  
`bf` can run in 2 modes; interpreter mode, or compiler mode. When compiling `bf` will output C from the given bf code, which can then be passed to any C99-or-later C compiler.

## Examples:
```sh
# runs in interpreter
bf i -c "++++"

# runs in interpreter, limited to 1000 interpreter cycles
bf i -c "+[]" -l 1000

# generates C output
bf c -c "+[]"

# generates C ouput from a file, to brot.c, and runs in 
# interpreter for 2 seconds to consteval data
bf c -O2 mandelbrot.bf -o brot.c
```
