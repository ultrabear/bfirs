#![warn(clippy::pedantic)]
#![allow(clippy::enum_glob_use)]

pub mod compiler;

use core::fmt;
use std::{
    fs::File,
    io::{self, Write},
    time::{Duration, Instant},
};

use compiler::{BfCompError, BfExecState, BfInstructionStream, BfOptimizable};

pub mod interpreter;

use either::Either;
use interpreter::{BfExecError, BfExecErrorTy, BrainFuckExecutor, BrainFuckExecutorBuilder};

use clap::{Args, Parser};

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

#[derive(Parser)]
/// a performance oriented brainfuck interpreter and compiler
struct TopLevel {
    #[command(subcommand)]
    sub: CompileSwitch,

    /// cellsize to use, defaults to 8, valid values: 8/16/32
    #[arg(short, long, global = true)]
    bits: Option<Mode>,

    /// number of cells to use, defaults to at least 30k
    #[arg(short, long, global = true)]
    size: Option<u32>,

    /// run the following argument as the input code
    #[arg(short, long, global = true)]
    code: Option<String>,

    /// file input of code
    #[arg(global = true)]
    file: Option<String>,
}

#[derive(clap::Subcommand)]
enum CompileSwitch {
    #[command(name = "interpret", visible_alias = "i")]
    Interpret(InterpreterArgs),
    #[command(name = "compile", visible_alias = "c")]
    Compile(CompilerArgs),
}

#[derive(Args, Copy, Clone)]
/// run brainfuck in an interpreter
struct InterpreterArgs {
    /// run a limited amount of instructions
    #[arg(short, long)]
    limit: Option<u64>,
}

#[derive(Args)]
/// compile brainfuck to C
struct CompilerArgs {
    /// output C to a file instead of stdout
    #[arg(short, long)]
    output: Option<String>,

    /// optimize by prerunning in interpreter for up to N seconds, defaults to 0
    #[arg(short = 'O', long = "opt-level")]
    opt_level: Option<u32>,
}

fn interpret<CellSize: BfOptimizable>(
    code: &[u8],
    arr_len: Option<u32>,
    args: InterpreterArgs,
) -> Result<(), Either<BfExecError, BfCompError>> {
    let code = BfInstructionStream::optimized_from_text(code.iter().copied(), arr_len)
        .map_err(Either::Right)?;

    let mut execenv =
        BrainFuckExecutor::new_stdio_locked::<CellSize>(code.reccomended_array_size());

    match args.limit {
        Some(lim) => {
            execenv.add_instruction_limit(lim).unwrap();
            execenv.run_limited(&code).map_err(Either::Left)?;
        }
        None => {
            execenv.run(&code).map_err(Either::Left)?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct ErrorReader;

impl io::Read for ErrorReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("ErrorReader always errors"))
    }
}

fn render_c_deadline<CellSize: BfOptimizable>(
    code: &BfInstructionStream<CellSize>,
    secs: u32,
    fp: &mut dyn io::Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut out = vec![];

    let mut execenv = BrainFuckExecutorBuilder::<CellSize, _, _>::new()
        .array_len(code.reccomended_array_size())
        .stream_in(ErrorReader)
        .stream_out(&mut out)
        .build()
        .unwrap();

    let est =
        u64::try_from(BrainFuckExecutor::<CellSize, ErrorReader, &mut Vec<u8>>::estimate_instructions_per_second(
        )).map_err(|_| "computer is too fast!! (u64::MAX overflowed when calculating instructions per second throughput)")? / 10;

    let start = std::time::Instant::now();
    let deadline = start + Duration::from_secs(u64::from(secs));

    execenv.add_instruction_limit(est)?;

    let mut s_idx = 0;

    loop {
        match execenv.run_limited_from(code, s_idx) {
            Ok(()) => {
                code.render_interpreted_c(
                    &BfExecState {
                        cursor: execenv.ptr,
                        data: &execenv.data,
                        instruction_pointer: None,
                    },
                    execenv.stdout,
                    fp,
                )?;
                break;
            }
            Err(BfExecError { source, idx }) => match source {
                err @ (BfExecErrorTy::Overflow
                | BfExecErrorTy::Underflow
                | BfExecErrorTy::InitOverflow) => {
                    return Err(format!("consteval: {err}").into());
                }
                BfExecErrorTy::IOError(_) => {
                    code.render_interpreted_c(
                        &BfExecState {
                            cursor: execenv.ptr,
                            data: &execenv.data,
                            instruction_pointer: Some(idx),
                        },
                        execenv.stdout,
                        fp,
                    )?;
                    break;
                }
                BfExecErrorTy::NotEnoughInstructions => {
                    s_idx = idx;

                    if Instant::now() > deadline {
                        code.render_interpreted_c(
                            &BfExecState {
                                cursor: execenv.ptr,
                                data: &execenv.data,
                                instruction_pointer: Some(idx),
                            },
                            execenv.stdout,
                            fp,
                        )?;
                        break;
                    }

                    execenv.add_instruction_limit(est)?;
                }
            },
        };
    }

    Ok(())
}

fn compile<CellSize: BfOptimizable>(
    code: &[u8],
    arr_len: Option<u32>,
    args: CompilerArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let code = BfInstructionStream::<CellSize>::optimized_from_text(code.iter().copied(), arr_len)?;

    let mut fp: Box<dyn io::Write> = match args.output {
        Some(fname) => Box::new(io::BufWriter::new(
            File::create(&fname).map_err(|e| PathIoError(fname, e))?,
        )),
        None => Box::new(io::BufWriter::new(io::stdout())),
    };

    match args.opt_level {
        None => code.render_c(&mut fp)?,
        Some(secs) => {
            if secs != 0 {
                render_c_deadline(&code, secs, &mut fp)?;
            } else {
                code.render_c(&mut fp)?;
            }
        }
    }

    fp.flush()?;

    Ok(())
}

#[derive(thiserror::Error, Debug)]
struct PathIoError(String, #[source] io::Error);

impl fmt::Display for PathIoError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.0, self.1)
    }
}

fn inner_main() -> Result<(), Box<dyn std::error::Error>> {
    let parse: TopLevel = TopLevel::parse();

    let TopLevel {
        sub,
        code,
        file,
        bits,
        size,
    } = parse;

    let code = match code {
        Some(code) => Vec::from(code),
        None => match file {
            Some(f) => std::fs::read(&f).map_err(|e| PathIoError(f, e))?,
            None => vec![],
        },
    };

    match sub {
        CompileSwitch::Compile(args) => match bits.unwrap_or(Mode::U8) {
            Mode::U8 => compile::<u8>(&code, size, args),
            Mode::U16 => compile::<u16>(&code, size, args),
            Mode::U32 => compile::<u32>(&code, size, args),
        }?,
        CompileSwitch::Interpret(args) => match bits.unwrap_or(Mode::U8) {
            Mode::U8 => interpret::<u8>(&code, size, args),
            Mode::U16 => interpret::<u16>(&code, size, args),
            Mode::U32 => interpret::<u32>(&code, size, args),
        }?,
    }

    Ok(())
}

fn main() {
    match inner_main() {
        Ok(()) => (),
        Err(e) => {
            // ignore all errors here, if we cant write to stdout/stderr its cooked anyways
            _ = io::stdout().flush();
            _ = writeln!(io::stderr(), "\x1b[91mERROR:\x1b[0m {e}");
            _ = io::stderr().flush();
        }
    }
}
