//! Implementation of minibit in-place interpreter
//! This interpreter trades execution speed for memory compaction,
//! allowing guaranteed memory use equal to the size of the input tape

use core::fmt;
use std::{collections::HashMap, io};

use crate::{
    compiler::{BfCompError, BfOptimizable},
    interpreter::BfExecError,
    state::BfState,
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
/// 6 -> zero
/// 7 -> wild
///
///
/// next 5 bits:
/// inc (how much to inc + 1)
/// dec (same as inc inverse)
/// incptr (how much to inc + 1)
/// decptr (same as incptr)
/// lstart (how much to jump forward, or if 0 lookup index in table)
/// lend (same as lstart for backward jump)
/// zero (no args)
/// wild (see WildArgs)
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
    Zero = 6,
    Wild = 7,
}

#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum WildArgs {
    Read = 0,
    Write = 1,
    IncPtrMany = 2,
    DecPtrMany = 3,
}

impl WildArgs {
    /// Converts a wild operand to the associated wild instruction
    unsafe fn from_wild(v: u8) -> Self {
        core::mem::transmute(v)
    }
}

impl Instr {
    fn with(self, operand: u8) -> u8 {
        (self as u8 & 7) << 5 | (operand & 31)
    }

    fn wild(args: WildArgs) -> u8 {
        Self::Wild.with(args as u8)
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
                6 => Self::Zero,
                7 => Self::Wild,
                _ => unreachable!(),
            },
            b & 31,
        )
    }
}

pub struct BTapeStream(Vec<BTape>, HashMap<usize, usize>);

struct DebugBTape<'a>(&'a [BTape]);

impl fmt::Debug for DebugBTape<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut l = f.debug_list();

        let mut idx = 0;

        while idx < self.0.len() {
            match Instr::decode(self.0[idx]) {
                (Instr::Wild, args) => {
                    let arg = unsafe { WildArgs::from_wild(args) };

                    match arg {
                        WildArgs::Read | WildArgs::Write => {
                            l.entry(&(Instr::Wild, arg));
                        }
                        WildArgs::IncPtrMany | WildArgs::DecPtrMany => {
                            let by = u64::from_le_bytes(
                                <[u8; 8]>::try_from(&self.0[idx + 1..idx + 9]).unwrap(),
                            );

                            idx += 8;
                            l.entry(&(Instr::Wild, arg, by));
                        }
                    }
                }
                (any, args) => {
                    l.entry(&(any, args));
                }
            }

            idx += 1;
        }

        l.finish()
    }
}

impl fmt::Debug for BTapeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BTapeStream")
            .field(&DebugBTape(&self.0))
            .field(&self.1)
            .finish()
    }
}

impl BTapeStream {
    fn rewrite(data: &[u8]) -> Vec<u8> {
        let mut out = vec![];

        let mut pull = data
            .iter()
            .filter(|v| matches!(v, b'+' | b'-' | b'>' | b'<' | b'[' | b']' | b',' | b'.'));

        macro_rules! push {
            ($byte:expr) => {{
                out.push($byte)
            }};
        }

        while let Some(next) = pull.next() {
            match next {
                b',' => push!(Instr::wild(WildArgs::Read)),
                b'.' => push!(Instr::wild(WildArgs::Write)),
                b'[' => {
                    let mut peek = pull.clone();

                    if let (Some(b'+' | b'-'), Some(b']')) = (peek.next(), peek.next()) {
                        push!(Instr::Zero.with(0));
                        pull = peek;
                    } else {
                        push!(Instr::LStart.with(0));
                    }
                }

                b']' => push!(Instr::LEnd.with(0)),

                initial @ (b'+' | b'-' | b'>' | b'<') => {
                    let mut count = 1u64;

                    let mut peek = pull.clone();

                    while Some(initial) == peek.next() {
                        count += 1;
                        pull = peek.clone();
                    }

                    let instr = match initial {
                        b'+' => Instr::Inc,
                        b'-' => Instr::Dec,
                        b'>' => Instr::IncPtr,
                        b'<' => Instr::DecPtr,
                        _ => unreachable!(),
                    };

                    let chunks = count / 32;
                    let last = (count % 32) as u8;

                    if chunks >= 1 && matches!(instr, Instr::IncPtr | Instr::DecPtr) {
                        if let Instr::IncPtr = instr {
                            push!(Instr::wild(WildArgs::IncPtrMany));
                            out.extend_from_slice(&count.to_le_bytes());
                        } else {
                            push!(Instr::wild(WildArgs::DecPtrMany));
                            out.extend_from_slice(&count.to_le_bytes());
                        }
                    } else {
                        for _ in 0..chunks {
                            push!(instr.with(31));
                        }

                        if last != 0 {
                            push!(instr.with(last - 1));
                        }
                    }
                }
                _ => {}
            }
        }

        out
    }

    fn insert_loop(data: &mut [BTape]) -> Result<HashMap<usize, usize>, BfCompError> {
        let mut stack = Vec::<usize>::new();
        let mut oversized = HashMap::new();

        let stream = data;
        let mut idx = 0;

        while idx < stream.len() {
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
                (Instr::Wild, arg)
                    if matches!(
                        unsafe { WildArgs::from_wild(arg) },
                        WildArgs::IncPtrMany | WildArgs::DecPtrMany
                    ) =>
                {
                    idx += 8
                }

                _ => {}
            }
            idx += 1;
        }

        if !stack.is_empty() {
            return Err(BfCompError::LoopCountMismatch);
        }

        Ok(oversized)
    }

    pub fn from_bf(data: &[u8]) -> Result<Self, BfCompError> {
        let mut data = Self::rewrite(data);

        let map = Self::insert_loop(&mut data)?;

        // NOTE: we are a safety invariant, a correctly constructed BTapeStream must have valid
        // WildArgs
        Ok(Self(data, map))
    }
}

impl BTapeStream {
    pub fn run<C: BfOptimizable, I: io::Read, O: io::Write>(
        &self,
        state: &mut BfState<C, I, O>,
    ) -> Result<(), BfExecError> {
        let mut idx = 0;

        while idx < self.0.len() {
            match Instr::decode(self.0[idx]) {
                (Instr::Zero, _) => state.zero(),
                (Instr::Inc, by) => state.inc(C::from(by).wrapping_add(C::from(1))),
                (Instr::Dec, by) => state.dec(C::from(by).wrapping_add(C::from(1))),
                (Instr::IncPtr, by) => {
                    state
                        .inc_ptr(by as usize + 1)
                        .map_err(|s| BfExecError { source: s, idx })?;
                }
                (Instr::DecPtr, by) => {
                    state
                        .dec_ptr(by as usize + 1)
                        .map_err(|s| BfExecError { source: s, idx })?;
                }
                (Instr::LStart, off) => {
                    if state.jump_forward() {
                        idx = if off != 0 {
                            idx + off as usize
                        } else {
                            self.1[&idx]
                        };
                    }
                }
                (Instr::LEnd, off) => {
                    if state.jump_backward() {
                        idx = if off != 0 {
                            idx - off as usize
                        } else {
                            self.1[&idx]
                        };
                    }
                }
                // SAFETY: A valid BTapeStream has valid WildArgs
                (Instr::Wild, kind) => match unsafe { WildArgs::from_wild(kind) } {
                    WildArgs::Read => {
                        state.read().map_err(|s| BfExecError { source: s, idx })?;
                    }
                    WildArgs::Write => {
                        state.write().map_err(|s| BfExecError { source: s, idx })?;
                    }
                    WildArgs::IncPtrMany => {
                        // SAFETY: Valid IncPtrMany has 8 LE bytes that encodes its operand
                        let operand = unsafe {
                            <[u8; 8]>::try_from(self.0.get_unchecked(idx + 1..idx + 9))
                                .unwrap_unchecked()
                        };

                        state
                            .inc_ptr(u64::from_le_bytes(operand) as usize)
                            .map_err(|s| BfExecError { source: s, idx })?;

                        idx += 8;
                    }
                    WildArgs::DecPtrMany => {
                        // SAFETY: Valid DecPtrMany has 8 LE bytes that encodes its operand
                        let operand = unsafe {
                            <[u8; 8]>::try_from(self.0.get_unchecked(idx + 1..idx + 9))
                                .unwrap_unchecked()
                        };

                        state
                            .dec_ptr(u64::from_le_bytes(operand) as usize)
                            .map_err(|s| BfExecError { source: s, idx })?;

                        idx += 8;
                    }
                },
            }

            idx += 1;
        }

        Ok(())
    }
}
