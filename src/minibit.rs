//! Implementation of minibit in-place interpreter
//! This interpreter trades execution speed for memory compaction,
//! allowing guaranteed memory use equal to the size of the input tape

use core::fmt;
use std::{collections::HashMap, io, time::Instant};

use crate::{
    compiler::{BfCompError, BfOptimizable},
    interpreter::{BfExecError, BfExecErrorTy},
};

/// BTape is a compacted form of bf executable tape
///
/// first 3 bits:
/// 0 -> inc
/// 1 -> dec
/// 2 -> incptr
/// 3 -> decptr
/// 4 -> lstart
/// 5 -> lend
/// 6 -> read/write
/// 7 -> zero
///
///
/// next 5 bits:
/// inc (how much to inc + 1)
/// dec (same as inc inverse)
/// incptr (how much to inc + 1)
/// decptr (same as incptr)
/// lstart (how much to jump forward, or if 0 lookup index in table)
/// lend (same as lstart for backward jump)
/// read/write (if 0 -> read, if 1 -> write)
/// zero (no args)
type BTape = u8;

#[repr(u8)]
#[derive(Copy, Clone, Debug)]
enum Instr {
    Inc = 0,
    Dec = 1,
    IncPtr = 2,
    DecPtr = 3,
    LStart = 4,
    LEnd = 5,
    ReadWrite = 6,
    Zero = 7,
}

impl Instr {
    fn with(self, operand: u8) -> u8 {
        (self as u8 & 7) << 5 | (operand & 31)
    }

    fn decode(b: u8) -> (Self, u8) {
        (
            match b >> 5 {
                0 => Self::Inc,
                1 => Self::Dec,
                2 => Self::IncPtr,
                3 => Self::DecPtr,
                4 => Self::LStart,
                5 => Self::LEnd,
                6 => Self::ReadWrite,
                7 => Self::Zero,
                _ => unreachable!(),
            },
            b & 31,
        )
    }
}

pub struct BTapeStream(Vec<BTape>, HashMap<usize, usize>);

impl fmt::Debug for BTapeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BTapeStream")
            .field(&self.0.iter().map(|i| Instr::decode(*i)).collect::<Vec<_>>())
            .field(&self.1)
            .finish()
    }
}

impl BTapeStream {
    fn retain_bf(data: &mut Vec<u8>) {
        data.retain(|v| matches!(v, b'+' | b'-' | b'>' | b'<' | b'[' | b']' | b',' | b'.'));
    }

    fn rewrite(data: &mut Vec<u8>) -> u64 {
        let mut push_idx = 0usize;
        let mut pull_idx = 0usize;
        let mut incptr_count = 0;

        macro_rules! push {
            ($byte:expr) => {{
                data[push_idx] = $byte;
                push_idx += 1;
            }};
        }

        while pull_idx < data.len() {
            match data[pull_idx] {
                b',' => push!(Instr::ReadWrite.with(0)),
                b'.' => push!(Instr::ReadWrite.with(1)),
                b'[' => {
                    if let Some(&[b'+' | b'-', b']']) = data.get(pull_idx + 1..pull_idx + 3) {
                        push!(Instr::Zero.with(0));
                        pull_idx += 2;
                    } else {
                        push!(Instr::LStart.with(0));
                    }
                }

                b']' => push!(Instr::LEnd.with(0)),

                initial @ (b'+' | b'-' | b'>' | b'<') => {
                    let mut count = 1u64;

                    while Some(&initial) == data.get(pull_idx + 1) {
                        count += 1;
                        pull_idx += 1;
                    }

                    let instr = match initial {
                        b'+' => Instr::Inc,
                        b'-' => Instr::Dec,
                        b'>' => {
                            incptr_count += count;
                            Instr::IncPtr
                        }
                        b'<' => Instr::DecPtr,
                        _ => unreachable!(),
                    };

                    let chunks = count / 32;
                    let last = (count % 32) as u8;

                    for _ in 0..chunks {
                        push!(instr.with(31));
                    }

                    if last != 0 {
                        push!(instr.with(last - 1));
                    }
                }
                _ => {}
            }

            pull_idx += 1;
        }

        data.truncate(push_idx);
        data.shrink_to_fit();

        incptr_count
    }

    fn insert_loop(data: &mut Vec<BTape>) -> Result<HashMap<usize, usize>, BfCompError> {
        let mut stack = Vec::<usize>::new();
        let mut oversized = HashMap::new();

        let stream = data;

        for idx in 0..stream.len() {
            // will not panic as we are iterating the stream length and never truncating
            #[allow(clippy::match_on_vec_items)]
            match Instr::decode(stream[idx]) {
                (Instr::LStart, _) => {
                    stack.push(idx);
                }
                (Instr::LEnd, _) => {
                    if let Some(v) = stack.pop() {
                        let distance = idx - v;

                        if distance <= 31 {
                            let distance = distance as u8;
                            stream[v] = Instr::LStart.with(distance);
                            stream[idx] = Instr::LEnd.with(distance);
                        } else {
                            stream[v] = Instr::LStart.with(0);
                            stream[idx] = Instr::LEnd.with(0);

                            oversized.insert(v, idx);
                            oversized.insert(idx, v);
                        }
                    } else {
                        return Err(BfCompError::LoopEndBeforeLoopStart);
                    }
                }
                _ => {}
            }
        }

        if !stack.is_empty() {
            return Err(BfCompError::LoopCountMismatch);
        }

        Ok(oversized)
    }

    pub fn from_bf(mut data: Vec<u8>) -> Result<(Self, u64), BfCompError> {
        Self::retain_bf(&mut data);

        let count = Self::rewrite(&mut data);

        let map = Self::insert_loop(&mut data)?;

        Ok((Self(data, map), count))
    }
}

pub struct BfTapeExecutor<T: BfOptimizable, I: io::Read, O: io::Write> {
    pub stdout: O,
    pub stdin: I,
    pub data: Box<[T]>,
    pub ptr: usize,
    pub last_flush: Instant,
}

impl<T: BfOptimizable, I: io::Read, O: io::Write> BfTapeExecutor<T, I, O> {
    unsafe fn get(&self) -> T {
        *self.data.get_unchecked(self.ptr)
    }

    unsafe fn set(&mut self, b: T) {
        *self.data.get_unchecked_mut(self.ptr) = b;
    }

    fn inc_ptr_by(&mut self, v: usize) -> Result<(), BfExecErrorTy> {
        self.ptr += v;
        if self.ptr >= self.data.len() {
            self.ptr -= v;
            return Err(BfExecErrorTy::Overflow);
        }
        Ok(())
    }

    fn dec_ptr_by(&mut self, v: usize) -> Result<(), BfExecErrorTy> {
        self.ptr = self.ptr.checked_sub(v).ok_or(BfExecErrorTy::Underflow)?;
        Ok(())
    }

    // inlining this increases performance on mandelbrot, probably thanks to reg cramming
    // im sorry clippy, the numbers are real this time
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn write(&mut self, v: u8) -> Result<(), BfExecErrorTy> {
        let _ = self.stdout.write(&[v])?;

        // based on 60 fps update (actual 62.5)
        if self.last_flush.elapsed().as_millis() > 16 {
            self.stdout.flush()?;
            self.last_flush = Instant::now();
        }

        Ok(())
    }

    fn read(&mut self) -> Result<u8, BfExecErrorTy> {
        // flush so the end user always gets prompts
        self.stdout.flush()?;

        let mut v = [0];
        let _ = self.stdin.read(&mut v)?;
        Ok(v[0])
    }

    pub fn run_stream(&mut self, stream: &BTapeStream) -> Result<(), BfExecError> {
        let mut idx = 0;

        // SAFETY: check ptr bounds now to ensure they are valid before a _unchecked op is called without a ptr mutating op
        if self.ptr >= self.data.len() {
            return Err(BfExecError {
                source: BfExecErrorTy::InitOverflow,
                idx,
            });
        }

        while idx < stream.0.len() {
            match Instr::decode(stream.0[idx]) {
                (Instr::Zero, _) => unsafe { self.set(T::ZERO) },
                (Instr::Inc, by) => unsafe {
                    self.set(
                        self.get()
                            .wrapping_add(T::from(by).wrapping_add(T::from(1))),
                    )
                },
                (Instr::Dec, by) => unsafe {
                    self.set(
                        self.get()
                            .wrapping_sub(T::from(by).wrapping_add(T::from(1))),
                    )
                },
                (Instr::IncPtr, by) => {
                    self.inc_ptr_by(by as usize + 1)
                        .map_err(|s| BfExecError { source: s, idx })?;
                }
                (Instr::DecPtr, by) => {
                    self.dec_ptr_by(by as usize + 1)
                        .map_err(|s| BfExecError { source: s, idx })?;
                }
                (Instr::LStart, off) => {
                    if unsafe { self.get() } == T::ZERO {
                        idx = if off != 0 {
                            idx + off as usize
                        } else {
                            stream.1[&idx]
                        };
                    }
                }
                (Instr::LEnd, off) => {
                    if unsafe { self.get() } != T::ZERO {
                        idx = if off != 0 {
                            idx - off as usize
                        } else {
                            stream.1[&idx]
                        };
                    }
                }
                (Instr::ReadWrite, kind) => {
                    if kind == 0 {
                        let val = self
                            .read()
                            .map_err(|s| BfExecError { source: s, idx })?
                            .into();
                        unsafe {
                            self.set(val);
                        }
                    } else {
                        self.write(unsafe { self.get().truncate_u8() })
                            .map_err(|s| BfExecError { source: s, idx })?;
                    }
                }
            }

            idx += 1;
        }

        Ok(())
    }
}
