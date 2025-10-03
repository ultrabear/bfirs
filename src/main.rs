#![warn(clippy::pedantic)]
#![allow(clippy::enum_glob_use)]

pub mod compiler;

use core::fmt;
use std::{
    fs::File,
    io::{self, Write},
    process::ExitCode,
    time::{Duration, Instant},
};

use clap_complete::{generate, Shell};
use compiler::{BfCompError, BfExecState, BfInstructionStream, BfOptimizable};

pub mod interpreter;
mod minibit;

use either::Either;
use interpreter::{BfExecError, BfExecErrorTy, BrainFuckExecutor, BrainFuckExecutorBuilder};

use clap::{Args, CommandFactory, Parser};

use crate::minibit::{BTapeStream, BfTapeExecutor};

#[derive(clap::ValueEnum, Clone, Copy)]
enum Mode {
    #[value(name = "8")]
    U8,
    #[value(name = "16")]
    U16,
    #[value(name = "32")]
    U32,
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum InterpreterType {
    Standard,
    Minibit,
}

#[derive(Parser)]
/// a performance oriented brainfuck interpreter and compiler
struct TopLevel {
    #[command(subcommand)]
    sub: CompileSwitch,

    /// cellsize to use
    #[arg(short, long, global = true, default_value = "8")]
    bits: Mode,

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
    #[command(name = "completions")]
    Completions(CompletionsArgs),
}

#[derive(Args)]
/// generate completions for a supported shell
struct CompletionsArgs {
    /// the shell to generate completions for
    shell: Shell,
}

#[derive(Args, Copy, Clone)]
/// run brainfuck in an interpreter
struct InterpreterArgs {
    /// run a limited amount of instructions
    #[arg(short, long)]
    limit: Option<u64>,

    /// Interpreter choice, the standard interpreter allocates approximately 10 bytes per byte,
    /// while the minibit interpreter allocates 1 byte per byte at most, but runs slower
    /// minibit also does not implement instruction limited mode
    #[arg(short, long, default_value = "standard")]
    interpreter: InterpreterType,
}

#[derive(Args)]
/// compile brainfuck to C
struct CompilerArgs {
    /// output C to a file instead of stdout
    #[arg(short, long)]
    output: Option<String>,

    /// consteval by prerunning in interpreter for up to N seconds, defaults to O1
    #[arg(short = 'O', long = "opt-level")]
    opt_level: Option<u32>,
}

/// Interprets in MiniBit runtime, a low memory overhead bf executor
fn minibit_interpret<C: BfOptimizable>(
    code: &[u8],
    arr_len: Option<u32>,
) -> Result<(), Either<BfExecError, BfCompError>> {
    let (arr_len, stream) = std::thread::scope(|s| {
        let arr_len = s.spawn(move || {
            arr_len.map_or_else(
                || std::cmp::max(bytecount::count(&code, b'>') as usize, 30_000),
                |v| v as usize,
            )
        });

        let stream = s.spawn(|| BTapeStream::from_bf(code));

        // these unwraps are fine as we dont expect either task to panic
        Ok((arr_len.join().unwrap(), stream.join().unwrap()?))
    })
    .map_err(Either::Right)?;

    let mut engine = BfTapeExecutor {
        stdout: std::io::stdout().lock(),
        stdin: std::io::stdin().lock(),
        data: vec![C::ZERO; arr_len].into_boxed_slice(),
        ptr: 0,
        last_flush: Instant::now(),
    };

    engine.run_stream(&stream).map_err(Either::Left)?;

    Ok(())
}

fn interpret<CellSize: BfOptimizable>(
    code: &[u8],
    arr_len: Option<u32>,
    args: InterpreterArgs,
) -> Result<(), Either<BfExecError, BfCompError>> {
    if matches!(args.interpreter, InterpreterType::Minibit) {
        return minibit_interpret::<CellSize>(code, arr_len);
    }

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
    let mut execenv = BrainFuckExecutorBuilder::<CellSize, _, _>::new()
        .array_len(code.reccomended_array_size())
        .stream_in(ErrorReader)
        .stream_out(vec![])
        .build()
        .unwrap();

    let est =
        u64::try_from(BrainFuckExecutor::<CellSize, ErrorReader, Vec<u8>>::estimate_instructions_per_second(
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
                    &execenv.stdout,
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
                // we know this cant be the Write impl, as Vec::write wont error
                BfExecErrorTy::IOError(_) => {
                    code.render_interpreted_c(
                        &BfExecState {
                            cursor: execenv.ptr,
                            data: &execenv.data,
                            instruction_pointer: Some(idx),
                        },
                        &execenv.stdout,
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
                            &execenv.stdout,
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

    let secs = args.opt_level.unwrap_or(1);

    if secs != 0 {
        render_c_deadline(&code, secs, &mut fp)?;
    } else {
        code.render_c(&mut *fp)?;
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

    let storage: Option<_>;
    let storage2: Option<_>;

    let code: &[u8] = match code {
        Some(code) => &Vec::from(code),
        None => match file {
            Some(f) => {
                let fp = std::fs::File::open(&f).map_err(|e| PathIoError(f.clone(), e))?;

                storage = Some(fp);

                let inserted = storage.as_ref().unwrap();

                // SAFETY: Please dont edit bf files while they are being compiled...
                let map = unsafe { memmap2::Mmap::map(&*inserted).map_err(|e| PathIoError(f, e))? };

                storage2 = Some(map);

                storage2.as_ref().unwrap().as_ref()
            }
            None => &[],
        },
    };

    match sub {
        CompileSwitch::Completions(args) => {
            let mut cmd = TopLevel::command();
            let cname = cmd.get_name().to_owned();

            let mut out = vec![];

            // dont write directly to stdout because clap_complete panics on io errors
            generate(args.shell, &mut cmd, cname, &mut out);

            io::stdout().write_all(&out)?;
        }
        CompileSwitch::Compile(args) => match bits {
            Mode::U8 => compile::<u8>(code, size, args),
            Mode::U16 => compile::<u16>(code, size, args),
            Mode::U32 => compile::<u32>(code, size, args),
        }?,
        CompileSwitch::Interpret(args) => match bits {
            Mode::U8 => interpret::<u8>(code, size, args),
            Mode::U16 => interpret::<u16>(code, size, args),
            Mode::U32 => interpret::<u32>(code, size, args),
        }?,
    }

    Ok(())
}

fn main() -> ExitCode {
    match inner_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // ignore all errors here, if we cant write to stdout/stderr its cooked anyways
            _ = io::stdout().flush();
            _ = writeln!(io::stderr(), "\x1b[91mERROR:\x1b[0m {e}");
            _ = io::stderr().flush();

            ExitCode::FAILURE
        }
    }
}
