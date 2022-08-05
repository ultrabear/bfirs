use std::io;

pub mod compiler;

use compiler::BfInstructionStream;

pub mod interpreter;

use interpreter::BrainFuckExecutor;

use std::fs::File;
use std::io::prelude::*;

use strum_macros::EnumString;

#[derive(EnumString, Clone, Copy)]
enum Mode {
	#[strum(serialize = "8")]
	U8,
	#[strum(serialize = "16")]
	U16,
	#[strum(serialize = "32")]
	U32,
}

use argh::{self, FromArgs};

#[derive(FromArgs)]
/// A low level brainfuck runtime.
struct ParseResult {
	/// read and run code from a given file
	#[argh(option, short = 'f')]
	file: Option<String>,

	/// read and run code from argv
	#[argh(option, short = 'a')]
	args: Option<String>,

	/// whether to use 8/16/32 bit mode, defaults to 8
	#[argh(option, short = 'm')]
	mode: Option<Mode>,
}

fn get_bf_from_argh() -> (Mode, Vec<u8>) {
	let res: ParseResult = argh::from_env();

	let mode = res.mode.unwrap_or(Mode::U8);

	if let Some(v) = res.file {
		let code_f = io::BufReader::new(
			File::open(&v)
				.map_err(|_| {
					eprintln!("\u{1b}[91mERROR\u{1b}[0m: Could not open file: {}", v);
					std::process::exit(1)
				})
				.unwrap(),
		);

		(mode, code_f.bytes().filter_map(|r| r.ok()).collect())
	} else {
		if let Some(v) = res.args {
			(mode, v.bytes().collect())
		} else {
			(mode, "".bytes().collect())
		}
	}
}

fn main() {
	let (mode, code) = get_bf_from_argh();

	macro_rules! run_different_sizes {
		($Ty:ty) => {{
			let code = BfInstructionStream::optimized_from_text(code.into_iter())
				.map_err(|e| {
					eprintln!("\u{1b}[91mERROR\u{1b}[0m: {}", e);
					std::process::exit(1)
				})
				.unwrap();

			let mut execenv = BrainFuckExecutor::new_stdio::<$Ty>(code.reccomended_array_size());

			execenv
				.run(&code)
				.map_err(|e| {
					eprintln!("\u{1b}[91mERROR\u{1b}[0m: {}", e);
					std::process::exit(1)
				})
				.unwrap();
		}};
	}

	match mode {
		Mode::U8 => {
			run_different_sizes!(u8)
		}
		Mode::U16 => {
			run_different_sizes!(u16)
		}
		Mode::U32 => {
			run_different_sizes!(u32)
		}
	}
}
