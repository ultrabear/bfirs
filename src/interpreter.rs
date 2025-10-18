use core::fmt;
use std::{
    hint::black_box,
    io::{self, StdinLock},
    time::Duration,
};
use thiserror::Error;

use crate::{
    compiler::BfOptimizable,
    nonblocking::{nonblocking, NonBlocking},
    state::BfState,
};

use super::compiler::BfInstruc;

#[derive(Debug, Error)]
pub struct BfExecError {
    pub source: BfExecErrorTy,
    pub idx: usize,
}

impl fmt::Display for BfExecError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.source, f)
    }
}

#[derive(Debug, Error)]
pub enum BfExecErrorTy {
    #[error("runtime overflowed its backing array")]
    Overflow,
    #[error("runtime underflowed its backing array")]
    Underflow,
    #[error("the pointer was already overflowed when the runtime started")]
    InitOverflow,
    #[error("not enough instructions to complete this task, halted before completion")]
    NotEnoughInstructions,
    #[error("an IO error was encountered {0:?}")]
    IOError(#[from] io::Error),
}

use std::time;

pub struct BrainFuckExecutor<T, I, O>
where
    O: io::Write,
    I: io::Read,
{
    pub state: BfState<T, I, O>,
    pub instruction_limit: u64,
}

pub fn new_stdio<T: BfOptimizable>(
    size: usize,
) -> Result<BrainFuckExecutor<T, StdinLock<'static>, NonBlocking>, BfExecError> {
    Ok(BrainFuckExecutor {
        state: BfState::new(
            0,
            vec![T::ZERO; size].into_boxed_slice(),
            io::stdin().lock(),
            nonblocking(io::stdout(), Duration::from_millis(10)).0,
        )
        .map_err(|_| BfExecError {
            source: BfExecErrorTy::InitOverflow,
            idx: 0,
        })?,
        instruction_limit: 0,
    })
}

#[derive(Debug, Error)]
pub struct Overflow;

impl fmt::Display for Overflow {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "overflow while attempting to add instruction limit")
    }
}

impl<T, I: io::Read, O: io::Write> BrainFuckExecutor<T, I, O> {
    /// Adds to instruction limit that is decremented each time `run_limited` is run
    ///
    /// # Errors
    /// This function will error if the instruction limit overflows `u64`
    pub fn add_instruction_limit(&mut self, amount: u64) -> Result<(), Overflow> {
        self.instruction_limit = self.instruction_limit.checked_add(amount).ok_or(Overflow)?;
        Ok(())
    }

    pub const fn instructions_left(&self) -> u64 {
        self.instruction_limit
    }
}

impl<T: BfOptimizable, I: io::Read, O: io::Write> BrainFuckExecutor<T, I, O> {
    // this inline(always) measurably increases performance (8.9s to 7.2s on mandelbrot) most probably
    // because if its not inlined it cant get enough context to optimize for what its being called
    // with (like the runtime const arguments that run and run_limited pass)
    #[inline(always)]
    fn internal_run<const LIMIT_INSTRUCTIONS: bool>(
        &mut self,
        stream: &[BfInstruc<T>],
        mut idx: usize,
    ) -> Result<(), BfExecError> {
        use BfInstruc::*;

        if LIMIT_INSTRUCTIONS && self.instruction_limit == 0 {
            return Err(BfExecError {
                source: BfExecErrorTy::NotEnoughInstructions,
                idx,
            });
        }

        // SAFETY: `ptr` bounds are checked by `ptr` mutating operations, so it will remain valid within this function
        while idx < stream.len() {
            if LIMIT_INSTRUCTIONS && self.instruction_limit == 0 {
                return Err(BfExecError {
                    source: BfExecErrorTy::NotEnoughInstructions,
                    idx,
                });
            }

            // TODO: try block :plead:
            (|| match stream[idx] {
                Zero => {
                    self.state.zero();
                    Ok(())
                }
                Inc => {
                    self.state.inc(1.into());
                    Ok(())
                }
                Dec => {
                    self.state.dec(1.into());
                    Ok(())
                }
                IncPtr => self.state.inc_ptr(1),
                DecPtr => self.state.dec_ptr(1),
                Write => self.state.write(),
                Read => self.state.read(),
                LStart(end) => {
                    if self.state.jump_forward() {
                        idx = end as usize;
                    }
                    Ok(())
                }
                LEnd(start) => {
                    if self.state.jump_backward() {
                        idx = start as usize;
                    }
                    Ok(())
                }
                IncBy(val) => {
                    self.state.inc(val);
                    Ok(())
                }
                DecBy(val) => {
                    self.state.dec(val);
                    Ok(())
                }
                IncPtrBy(val) => self.state.inc_ptr(val.get() as usize),
                DecPtrBy(val) => self.state.dec_ptr(val.get() as usize),
            })()
            .map_err(|source| BfExecError { source, idx })?;

            idx += 1;

            if LIMIT_INSTRUCTIONS {
                self.instruction_limit -= 1;
            }
        }

        Ok(())
    }

    /// Runs brainfuck stream unbounded, this function is not guaranteed to halt.
    ///
    /// # Errors
    /// This function will error if there is an error in the in/out streams or if the data pointer overflows/underflows.
    pub fn run(&mut self, stream: &[BfInstruc<T>]) -> Result<(), BfExecError> {
        self.internal_run::<false>(stream, 0)
    }

    /// Runs brainfuck with a limited instruction count specified by [`BrainFuckExecutor::instructions_left`], this function will eventually halt.
    ///
    /// If the brainfuck finishes executing without reaching the limit, the leftover instructions will be kept in instructions left, while if it errors instructions left will be zero.
    ///
    /// # Errors
    /// This function will error if there is an error in the in/out streams, if the data pointer overflows/underflows, or if the instruction limit is reached before execution ends.
    pub fn run_limited(&mut self, stream: &[BfInstruc<T>]) -> Result<(), BfExecError> {
        self.internal_run::<true>(stream, 0)
    }

    /// Runs brainfuck with a limited instruction count specified by [`BrainFuckExecutor::instructions_left`], this function will eventually halt.
    ///
    /// If the brainfuck finishes executing without reaching the limit, the leftover instructions will be kept in instructions left, while if it errors instructions left will be zero.
    ///
    /// This function accepts a start parameter that tells it to start from a specific index in the
    /// stream, this allows for completely pausing and restarting execution of code
    ///
    /// # Errors
    /// This function will error if there is an error in the in/out streams, if the data pointer overflows/underflows, or if the instruction limit is reached before execution ends.
    pub fn run_limited_from(
        &mut self,
        stream: &[BfInstruc<T>],
        start: usize,
    ) -> Result<(), BfExecError> {
        self.internal_run::<true>(stream, start)
    }

    /// provides a calculated at runtime estimate of instruction throughput for the given mode using 100k iterations,
    /// does not take cache locality into account so will likely return higher numbers than real world data
    #[must_use]
    // this will not panic: the instructions will infinitely loop without overflowing or
    // underflowing the pointer
    #[allow(clippy::missing_panics_doc)]
    pub fn estimate_instructions_per_second() -> u128 {
        Self::estimate_instructions_per_second_from_stream(&[
            BfInstruc::Inc,
            BfInstruc::LStart(5),
            BfInstruc::IncPtr,
            BfInstruc::Dec,
            BfInstruc::Dec,
            BfInstruc::IncBy(T::from(4)),
            BfInstruc::DecPtr,
            BfInstruc::LEnd(1),
        ])
        .unwrap()
    }

    /// Estimates instructions per second from a provided stream, doing up to 100k iterations
    ///
    /// # Errors
    /// This function will error if the passed brainfuck stream causes a underflow or overflow
    // this will not panic: all required arguments have been provided to the builder
    #[allow(clippy::missing_panics_doc)]
    pub fn estimate_instructions_per_second_from_stream(
        stream: &[BfInstruc<T>],
    ) -> Result<u128, BfExecError> {
        const SAMPLE: u32 = 100_000;

        let mut exec = BrainFuckExecutor {
            state: BfState::new(
                0,
                vec![T::ZERO; 30_000].into_boxed_slice(),
                io::empty(),
                io::sink(),
            )
            .unwrap_or_else(|_| panic!()),
            instruction_limit: SAMPLE.into(),
        };

        let start = time::Instant::now();

        // black_box stream so its not const folded
        if let Err(e) = exec.run_limited(black_box(stream)) {
            match e {
                BfExecError {
                    source: BfExecErrorTy::NotEnoughInstructions,
                    ..
                } => {}
                v => return Err(v),
            }
        };

        // black_box after running so that the exec environment must have been modified
        let exec = black_box(exec);

        Ok(
            (u128::from(SAMPLE - u32::try_from(exec.instructions_left()).unwrap()) * 1_000_000_000)
                / start.elapsed().as_nanos(),
        )
    }
}

#[test]
fn test_exec_env() {
    use super::compiler::BfInstructionStream;

    let parse_bf =
        |code: &str| BfInstructionStream::optimized_from_text(code.bytes(), None).unwrap();

    let run_code = |x: &str| {
        let mut env = new_stdio::<u8>(30_000).expect("Nonzero");

        env.run(&parse_bf(x)).unwrap();
    };

    let expect_output = |code: &str, expect: &str| {
        let mut outv = Vec::new();

        let mut env = BrainFuckExecutor {
            state: BfState::new(
                0,
                vec![0u8; 30_000].into_boxed_slice(),
                io::empty(),
                &mut outv,
            )
            .map_err(|_| ())
            .unwrap(),
            instruction_limit: 0,
        };
        env.run(&parse_bf(code)).unwrap();

        if outv != expect.as_bytes() {
            panic!("Expected {}, got instead {:?}", expect, outv);
        }
    };

    macro_rules! expect_error {
        ($s:expr, $err:pat, $rep:expr) => {
            let mut env = new_stdio::<u8>(30_000).expect("Nonzero");

            env.add_instruction_limit(1_000_000).unwrap();

            match env.run_limited(&parse_bf($s)) {
                Ok(_) => panic!("Got Ok(()) value, expected {:?}", $rep),
                Err(err) => match err {
                    BfExecError { source: $err, .. } => (),
                    e => panic!("Got {:?} value, expected {:?}", e, $rep),
                },
            };
        };
    }

    expect_output("++++[>++++[>++++<-]<-]>>+.", "A");
    expect_error!("<", BfExecErrorTy::Underflow, BfExecErrorTy::Underflow);
    expect_error!("+[>+]", BfExecErrorTy::Overflow, BfExecErrorTy::Overflow);
    expect_error!(
        "+[]",
        BfExecErrorTy::NotEnoughInstructions,
        BfExecErrorTy::NotEnoughInstructions
    );
    run_code("-");
    run_code(">>");
}
