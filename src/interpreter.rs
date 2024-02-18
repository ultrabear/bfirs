use core::fmt;
use std::{hint::black_box, io};
use thiserror::Error;

use crate::compiler::BfOptimizable;

use super::compiler::BfInstruc;

#[derive(Error, Debug, Copy, Clone)]
pub enum ExecutorBuilderError {
    #[error("no input stream was specified")]
    NoStreamIn,
    #[error("no output stream was specified")]
    NoStreamOut,
    #[error("no array size was specified")]
    NoArraySize,
}

pub struct BrainFuckExecutorBuilder<T, I, O> {
    stdout: Option<O>,
    stdin: Option<I>,
    array_len: Option<usize>,
    starting_ptr: Option<usize>,
    fill: Option<T>,
    instruction_limit: Option<u64>,
}

impl<T: Clone, I: io::Read, O: io::Write> Default for BrainFuckExecutorBuilder<T, I, O> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone, I: io::Read, O: io::Write> BrainFuckExecutorBuilder<T, I, O> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stdout: None,
            stdin: None,
            array_len: None,
            starting_ptr: None,
            fill: None,
            instruction_limit: None,
        }
    }

    /// Builds the executor
    ///
    /// # Errors
    /// This function will error if no stream in/out is specified or if no array size is specified
    pub fn build(self) -> Result<BrainFuckExecutor<T, I, O>, ExecutorBuilderError>
    where
        T: Default,
    {
        use ExecutorBuilderError::{NoArraySize, NoStreamIn, NoStreamOut};

        let s_out = self.stdout.ok_or(NoStreamOut)?;
        let s_in = self.stdin.ok_or(NoStreamIn)?;
        let array_len = self.array_len.ok_or(NoArraySize)?;

        Ok(BrainFuckExecutor {
            data: std::iter::repeat(self.fill.unwrap_or_default())
                .take(array_len)
                .collect(),
            stdin: s_in,
            stdout: s_out,
            ptr: self.starting_ptr.unwrap_or(0),
            last_flush: time::Instant::now(),
            instruction_limit: self.instruction_limit.unwrap_or(0),
        })
    }

    #[must_use]
    pub fn stream_in(mut self, s: I) -> Self {
        self.stdin = Some(s);

        self
    }

    #[must_use]
    pub fn stream_out(mut self, s: O) -> Self {
        self.stdout = Some(s);

        self
    }

    #[must_use]
    pub fn fill(mut self, fill: T) -> Self {
        self.fill = Some(fill);

        self
    }

    #[must_use]
    pub const fn array_len(mut self, v: usize) -> Self {
        self.array_len = Some(v);

        self
    }

    #[must_use]
    pub const fn starting_ptr(mut self, ptr: usize) -> Self {
        self.starting_ptr = Some(ptr);

        self
    }

    #[must_use]
    pub const fn limit(mut self, limit: u64) -> Self {
        self.instruction_limit = Some(limit);

        self
    }
}

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
    pub stdout: O,
    pub stdin: I,
    pub data: Box<[T]>,
    pub ptr: usize,
    pub last_flush: time::Instant,
    pub instruction_limit: u64,
}

impl BrainFuckExecutor<(), io::Stdin, io::Stdout> {
    #[allow(clippy::missing_panics_doc)]
    #[must_use]
    pub fn new_stdio<T: Clone + Default>(
        array_len: usize,
    ) -> BrainFuckExecutor<T, io::Stdin, io::Stdout> {
        BrainFuckExecutorBuilder::new()
            .stream_in(io::stdin())
            .stream_out(io::stdout())
            .array_len(array_len)
            .build()
            // This panic should not occur because the builder has been constructed with at least the minimum amount of required fields
            .expect("this panic should not occur, minimum builder fields are present")
    }

    #[allow(clippy::missing_panics_doc)]
    #[must_use]
    pub fn new_stdio_locked<'i, 'o, T: Clone + Default>(
        array_len: usize,
    ) -> BrainFuckExecutor<T, io::StdinLock<'i>, io::StdoutLock<'o>> {
        BrainFuckExecutorBuilder::new()
            .stream_in(io::stdin().lock())
            .stream_out(io::stdout().lock())
            .array_len(array_len)
            .build()
            // This panic should not occur because the builder has been constructed with at least the minimum amount of required fields
            .expect("this panic should not occur, minimum builder fields are present")
    }
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

    pub const fn state(&self) -> (usize, &[T]) {
        (self.ptr, &self.data)
    }

    pub fn state_mut(&mut self) -> (&mut usize, &mut [T]) {
        (&mut self.ptr, &mut self.data)
    }

    pub fn destructure(self) -> (usize, Box<[T]>, I, O) {
        (self.ptr, self.data, self.stdin, self.stdout)
    }
}

impl<T: BfOptimizable, I: io::Read, O: io::Write> BrainFuckExecutor<T, I, O> {
    #[inline]
    unsafe fn cur_unchecked(&self) -> T {
        // SAFETY: The caller has asserted that the current pointer is a valid index
        debug_assert!(self.ptr < self.data.len());
        *self.data.get_unchecked(self.ptr)
    }

    #[inline]
    unsafe fn map_current(&mut self, func: impl FnOnce(T) -> T) {
        // SAFETY: The caller has asserted that the current pointer is a valid index
        debug_assert!(self.ptr < self.data.len());
        *self.data.get_unchecked_mut(self.ptr) = func(self.cur_unchecked());
    }

    #[inline]
    fn inc_ptr_by(&mut self, v: usize) -> Result<(), BfExecErrorTy> {
        self.ptr += v;
        if self.ptr >= self.data.len() {
            self.ptr -= v;
            return Err(BfExecErrorTy::Overflow);
        }
        Ok(())
    }

    #[inline]
    fn dec_ptr_by(&mut self, v: usize) -> Result<(), BfExecErrorTy> {
        self.ptr = self.ptr.checked_sub(v).ok_or(BfExecErrorTy::Underflow)?;
        Ok(())
    }

    #[inline]
    fn write(&mut self, v: u8) -> Result<(), BfExecErrorTy> {
        let _ = self.stdout.write(&[v])?;

        // based on 60 fps update (actual 62.5)
        if self.last_flush.elapsed().as_millis() > 16 {
            self.stdout.flush()?;
            self.last_flush = time::Instant::now();
        }

        Ok(())
    }

    #[inline]
    fn read(&mut self) -> Result<u8, BfExecErrorTy> {
        // flush so the end user always gets prompts
        self.stdout.flush()?;

        let mut v = [0];
        let _ = self.stdin.read(&mut v)?;
        Ok(v[0])
    }

    fn internal_run<const LIMIT_INSTRUCTIONS: bool>(
        &mut self,
        stream: &[BfInstruc<T>],
        mut idx: usize,
    ) -> Result<(), BfExecError> {
        use BfInstruc::*;

        let len = stream.len();

        // SAFETY: check ptr bounds now to ensure they are valid before a _unchecked op is called without a ptr mutating op
        if self.ptr >= self.data.len() {
            return Err(BfExecError {
                source: BfExecErrorTy::InitOverflow,
                idx,
            });
        }

        if LIMIT_INSTRUCTIONS && self.instruction_limit == 0 {
            return Err(BfExecError {
                source: BfExecErrorTy::NotEnoughInstructions,
                idx,
            });
        }

        // SAFETY: `ptr` bounds are checked by `ptr` mutating operations, so it will remain valid within this function
        while idx < len {
            if LIMIT_INSTRUCTIONS && self.instruction_limit == 0 {
                return Err(BfExecError {
                    source: BfExecErrorTy::NotEnoughInstructions,
                    idx,
                });
            }

            unsafe {
                // TODO: try block :plead:
                (|| match stream[idx] {
                    Zero => {
                        self.map_current(|_| T::ZERO);
                        Ok(())
                    }
                    Inc => {
                        self.map_current(|c| c.wrapping_add(T::from(1)));
                        Ok(())
                    }
                    Dec => {
                        self.map_current(|c| c.wrapping_sub(T::from(1)));
                        Ok(())
                    }
                    IncPtr => self.inc_ptr_by(1),
                    DecPtr => self.dec_ptr_by(1),
                    Write => self.write(self.cur_unchecked().truncate_u8()),
                    Read => {
                        let v = self.read()?.into();
                        self.map_current(|_| v);
                        Ok(())
                    }
                    LStart(end) => {
                        if self.cur_unchecked() == T::ZERO {
                            idx = end as usize;
                        }
                        Ok(())
                    }
                    LEnd(start) => {
                        if self.cur_unchecked() != T::ZERO {
                            idx = start as usize;
                        }
                        Ok(())
                    }
                    IncBy(val) => {
                        self.map_current(|c| c.wrapping_add(val));
                        Ok(())
                    }
                    DecBy(val) => {
                        self.map_current(|c| c.wrapping_sub(val));
                        Ok(())
                    }
                    IncPtrBy(val) => self.inc_ptr_by(val.get() as usize),
                    DecPtrBy(val) => self.dec_ptr_by(val.get() as usize),
                })()
                .map_err(|source| BfExecError { source, idx })?;
            }

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

        let mut exec = BrainFuckExecutorBuilder::<T, io::Empty, io::Sink>::new()
            .stream_in(io::empty())
            .stream_out(io::sink())
            .array_len(30_000)
            .limit(SAMPLE.into())
            .build()
            .unwrap();

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
        let mut env = BrainFuckExecutor::new_stdio::<u8>(30_000);

        env.run(&parse_bf(x)).unwrap();
    };

    let expect_output = |code: &str, expect: &str| {
        let mut outv = Vec::new();

        let mut env = BrainFuckExecutorBuilder::<u8, _, _>::new()
            .stream_in(io::empty())
            .stream_out(&mut outv)
            .array_len(30_000)
            .build()
            .unwrap();

        env.run(&parse_bf(code)).unwrap();

        if outv != expect.as_bytes() {
            panic!("Expected {}, got instead {:?}", expect, outv);
        }
    };

    macro_rules! expect_error {
        ($s:expr, $err:pat, $rep:expr) => {
            let mut env = BrainFuckExecutor::new_stdio::<u8>(30_000);

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
