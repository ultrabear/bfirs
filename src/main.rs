use std::io;

pub mod compiler;

use compiler::BfInstructionStream;

pub mod interpreter;

use interpreter::BrainFuckExecutor;

use std::fs::File;
use std::io::prelude::*;

use clap::{arg, value_parser, Arg, Command, ValueEnum};

#[derive(ValueEnum, Clone, Copy)]
enum Mode {
	#[clap(name = "8")]
	U8,
	#[clap(name = "16")]
	U16,
	#[clap(name = "32")]
	U32,
}

fn get_bf_from_args() -> (Mode, Vec<u8>) {
	let parsed = Command::new("bfirs")
		.about("A low level brainfuck runtime")
		.arg(arg!(-f --file [FILE] "Read and run code from a given file"))
		.arg(arg!(-a --args [BRAINFUCK] "Read and run code from argv"))
		.arg(
			Arg::new("mode")
				.long("mode")
				.short('m')
				.takes_value(true)
				.value_name("mode")
				.value_parser(value_parser!(Mode))
				.help("Whether to use 8/16/32 bit mode, defaults to 8"),
		)
		.get_matches();

	let mut mode = Mode::U8;

	if let Some(m) = parsed.get_one::<Mode>("mode") {
		mode = *m;
	}

	if let Some(v) = parsed.get_one::<String>("file") {
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
		if let Some(v) = parsed.get_one::<String>("args") {
			(mode, v.bytes().collect())
		} else {
			(mode, "".bytes().collect())
		}
	}
}

fn main() {
	let (mode, code) = get_bf_from_args();

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
