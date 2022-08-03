use std::io;

pub mod compiler;

use compiler::{BfCompError, BfInstruc, BfInstructionStream, BfOptimizable};

pub mod interpreter;

use interpreter::BrainFuckExecutor;

use io::prelude::*;
use std::fs::File;

//use clap::{Arg, App};

fn get_bf_from_args<T: BfOptimizable>() -> Result<BfInstructionStream<T>, BfCompError> {
	let code_f = io::BufReader::new(File::open("code.bf").unwrap());

	BfInstructionStream::optimized_from_text(code_f.bytes().filter_map(|r| r.ok()))
}

fn main() {
	let code = get_bf_from_args().unwrap();

	let array_len: u32 = code
		.iter()
		.fold(0, |accu, x| {
			if let BfInstruc::IncPtr = x {
				accu + 1
			} else {
				accu
			}
		})
		.max(30_000);

	let mut execenv = BrainFuckExecutor::new_stdio::<u8>(
		array_len
			.try_into()
			.expect("16 bit platforms not supported"),
	);

	execenv.run(&code).unwrap();
}
