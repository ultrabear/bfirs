//! An intermediate DAG representation for a BF programs optimization stage

use std::{collections::HashMap, io, ops::Range};

use crate::{
    compiler::{BfCompError, BfOptimizable},
    interpreter::{BfExecError, BfExecErrorTy},
    state,
};

pub enum Token {
    Zero,
    Inc(u32),
    Dec(u32),
    IncPtr(usize),
    DecPtr(usize),
    Read,
    Write,
    LStart,
    LEnd,
}

impl Token {
    pub fn parse(data: &[u8]) -> Vec<Self> {
        let mut valid = data.iter().filter(|b| b"+-><[],.".contains(b)).copied();

        let mut out = vec![];

        while let Some(byte) = valid.next() {
            match byte {
                b'.' => out.push(Self::Write),
                b',' => out.push(Self::Read),
                b'[' => {
                    let mut peek = valid.clone();

                    if let (Some(b'+' | b'-'), Some(b']')) = (peek.next(), peek.next()) {
                        out.push(Self::Zero);
                        valid = peek;
                    } else {
                        out.push(Self::LStart);
                    }
                }
                b']' => out.push(Self::LEnd),
                initial @ (b'+' | b'-' | b'>' | b'<') => {
                    let mut count = 1usize;

                    let mut peek = valid.clone();

                    while Some(initial) == peek.next() {
                        count += 1;
                        valid = peek.clone();
                    }

                    out.push(match initial {
                        b'+' => Self::Inc(count as u32),
                        b'-' => Self::Dec(count as u32),
                        b'>' => Self::IncPtr(count),
                        b'<' => Self::DecPtr(count),
                        _ => unreachable!(),
                    });
                }
                _ => unreachable!(),
            }
        }

        out
    }

    pub fn to_tree(this: &[Self]) -> Result<Vec<ITree>, BfCompError> {
        let mut out = vec![];
        let mut ctx: Vec<usize> = vec![];

        let mut ptr = &mut out;

        macro_rules! push {
            ($e:expr) => {
                ptr.push($e)
            };
        }

        for tok in this {
            match tok {
                Token::Zero => push!(ITree::Zero),
                Token::Inc(by) => push!(ITree::Inc(*by)),
                Token::Dec(by) => push!(ITree::Dec(*by)),
                Token::IncPtr(by) => push!(ITree::IncPtr(*by)),
                Token::DecPtr(by) => push!(ITree::DecPtr(*by)),
                Token::Read => push!(ITree::Read),
                Token::Write => push!(ITree::Write),
                Token::LStart => {
                    let idx = ptr.len();
                    ptr.push(ITree::Loop(vec![]));
                    ctx.push(idx);

                    let ITree::Loop(nptr) = &mut ptr[idx] else {
                        unreachable!()
                    };

                    ptr = nptr;
                }
                Token::LEnd => {
                    let Some(_) = ctx.pop() else {
                        return Err(BfCompError::LoopEndBeforeLoopStart);
                    };

                    let mut nptr = &mut out;

                    for idx in &ctx {
                        let (ITree::Loop(data) | ITree::WriteLoop(data)) = &mut nptr[*idx] else {
                            unreachable!()
                        };

                        nptr = data;
                    }

                    ptr = nptr;
                }
            }
        }

        if !ctx.is_empty() {
            return Err(BfCompError::LoopCountMismatch);
        }

        Ok(out)
    }
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct MulArg {
    offset: isize,
    change: i64,
}

#[derive(Debug)]
pub enum ITree {
    Zero,
    Mul(Range<isize>, Vec<MulArg>),
    //ZeroRange(u32),
    Inc(u32),
    Dec(u32),
    IncPtr(usize),
    DecPtr(usize),
    Read,
    Write,
    Loop(Vec<ITree>),
    If(Vec<ITree>),
    WriteLoop(Vec<ITree>),
}

impl ITree {
    fn terminates(&self) -> bool {
        !matches!(self, Self::Loop(_) | Self::WriteLoop(_))
    }

    fn zero_in_loop(this: &[Self]) -> bool {
        matches!(this, [Self::Inc(1)] | [Self::Dec(1)])
    }

    fn terminating_nested_len(this: &[Self]) -> usize {
        this.len()
            + this
                .iter()
                .map(|v| {
                    if let Self::If(children) = v {
                        Self::terminating_nested_len(children)
                    } else {
                        0
                    }
                })
                .sum::<usize>()
    }

    fn is_writeloop(this: &[Self]) -> bool {
        Self::terminating_nested_len(this) < 32
            && this
                .iter()
                .all(|c| c.terminates() & !matches!(c, Self::Read))
            && this.iter().any(|c| matches!(c, Self::Write))
    }

    fn as_multiply(this: &[Self]) -> Option<(Range<isize>, Vec<MulArg>)> {
        const Z_OFFSET: usize = 32;

        let mut minivm = [0i64; 64];
        let mut idx = Z_OFFSET;

        let mut bounds = 0..0isize;

        for node in this {
            match node {
                Self::Zero
                | Self::Mul(_, _)
                | Self::Read
                | Self::Write
                | Self::Loop(_)
                | Self::WriteLoop(_)
                | Self::If(_) => return None,
                Self::Inc(by) => {
                    let Some(inc) = minivm[idx].checked_add(i64::from(*by)) else {
                        return None;
                    };

                    minivm[idx] = inc;
                }
                Self::Dec(by) => {
                    let Some(dec) = minivm[idx].checked_sub(i64::from(*by)) else {
                        return None;
                    };

                    minivm[idx] = dec;
                }
                Self::IncPtr(by) => {
                    let Some(incptr) = idx.checked_add(*by) else {
                        return None;
                    };

                    if incptr < minivm.len() {
                        idx = incptr;

                        bounds.end = core::cmp::max(bounds.end, (idx as isize) - Z_OFFSET as isize);
                    } else {
                        return None;
                    }
                }
                Self::DecPtr(by) => {
                    let Some(decptr) = idx.checked_sub(*by) else {
                        return None;
                    };

                    idx = decptr;

                    bounds.start = core::cmp::min(bounds.start, (idx as isize) - Z_OFFSET as isize);
                }
            }
        }

        if (idx == Z_OFFSET) & (minivm[Z_OFFSET] == -1) {
            let mut out = vec![];

            for (idx, cell) in minivm.into_iter().enumerate() {
                if (cell != 0) & (idx != Z_OFFSET) {
                    out.push(MulArg {
                        offset: idx as isize - Z_OFFSET as isize,
                        change: cell,
                    });
                }
            }

            Some((bounds, out))
        } else {
            None
        }
    }

    fn synth_inner(this: &[Self], stream: &mut Vec<Executable>, cache: &mut MultiplyCache) {
        for node in this {
            match node {
                ITree::Zero => stream.push(Executable::Zero),
                ITree::Mul(range, mul_args) => {
                    let i = cache.insert(DistinctMultiply(range.clone(), mul_args.clone()));

                    stream.push(Executable::Multiply(i));
                }
                ITree::Inc(by) => stream.push(Executable::Inc(*by)),
                ITree::Dec(by) => stream.push(Executable::Dec(*by)),
                ITree::IncPtr(by) => stream.push(Executable::IncPtr(*by as u32)),
                ITree::DecPtr(by) => stream.push(Executable::DecPtr(*by as u32)),
                ITree::Read => stream.push(Executable::Read),
                ITree::Write => stream.push(Executable::Write),
                //  ITree::ZeroRange(by) => todo!(), // stream.push(Executable::ZeroRange(*by)),
                ITree::Loop(itrees) => {
                    let s_idx = stream.len();
                    stream.push(Executable::LStart(0));
                    Self::synth_inner(&itrees, stream, cache);

                    let e_idx = if let Some(Executable::LEnd(_)) = stream.last() {
                        stream.len() - 1
                    } else {
                        let e_idx = stream.len();
                        stream.push(Executable::LEnd(s_idx as u32));
                        e_idx
                    };

                    stream[s_idx] = Executable::LStart(e_idx as u32);
                }
                ITree::WriteLoop(itrees) => {
                    let s_idx = stream.len();
                    stream.push(Executable::WLStart(0));
                    Self::synth_inner(&itrees, stream, cache);
                    let e_idx = stream.len();
                    stream.push(Executable::WLEnd(s_idx as u32));

                    stream[s_idx] = Executable::WLStart(e_idx as u32);
                }
                ITree::If(itrees) => {
                    let s_idx = stream.len();
                    stream.push(Executable::LStart(0));
                    Self::synth_inner(&itrees, stream, cache);
                    let e_idx = stream.len() - 1;

                    if e_idx == s_idx {
                        stream.pop();
                    } else {
                        stream[s_idx] = Executable::LStart(e_idx as u32);
                    }
                }
            }
        }
    }

    pub fn synthesize(this: &[Self]) -> InterpreterStream {
        let mut cache = MultiplyCache::default();

        let mut stream = vec![];

        Self::synth_inner(this, &mut stream, &mut cache);

        InterpreterStream(stream, cache.0)
    }
}

pub fn rewrite_zero(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(children) = node {
            if ITree::zero_in_loop(&children) {
                *node = ITree::Zero;
            } else {
                rewrite_zero(children);
            }
        }
    }
}

pub fn find_if_conditions(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(ref mut children) = node {
            find_if_conditions(children);

            if let Some(ITree::Zero) = children.last() {
                *node = ITree::If(core::mem::take(children));
            }
        }
    }
}

pub fn rewrite_multiply(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(children) = node {
            if let Some(mulargs) = ITree::as_multiply(children) {
                *node = ITree::Mul(mulargs.0, mulargs.1);
            } else {
                rewrite_multiply(children);
            }
        }
    }
}

pub fn rewrite_write_loops(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(children) = node {
            if ITree::is_writeloop(&children) {
                *node = ITree::WriteLoop(core::mem::take(children));
            } else {
                rewrite_write_loops(children);
            }
        }
    }
}

#[derive(Debug)]
pub enum Executable {
    Zero,
    Inc(u32),
    Dec(u32),
    IncPtr(u32),
    DecPtr(u32),
    WLStart(u32),
    WLEnd(u32),
    LStart(u32),
    LEnd(u32),
    Read,
    Write,
    Multiply(u32),
    //    ZeroRange(u32),
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct DistinctMultiply(Range<isize>, Vec<MulArg>);

#[derive(Default)]
pub struct MultiplyCache(Vec<DistinctMultiply>, HashMap<DistinctMultiply, u32>);

impl MultiplyCache {
    fn insert(&mut self, dm: DistinctMultiply) -> u32 {
        if let Some(v) = self.1.get(&dm) {
            *v
        } else {
            let idx = self.0.len() as u32;

            self.1.insert(dm.clone(), idx);
            self.0.push(dm);

            idx
        }
    }
}

#[derive(Debug)]
pub struct InterpreterStream(Vec<Executable>, Vec<DistinctMultiply>);

impl InterpreterStream {
    fn write<C: BfOptimizable, I: io::Read, O: io::Write, const BUF: usize>(
        buf: &mut [u8; BUF],
        cursor: &mut usize,
        state: &mut state::BfState<C, I, O>,
    ) -> Result<(), BfExecErrorTy> {
        if *cursor == buf.len() {
            state.write.write_all(buf)?;
            *cursor = 0;
        }

        buf[*cursor] = state.get().truncate_u8();
        *cursor += 1;

        Ok(())
    }

    fn softflush<C: BfOptimizable, I: io::Read, O: io::Write, const BUF: usize>(
        buf: &mut [u8; BUF],
        cursor: &mut usize,
        state: &mut state::BfState<C, I, O>,
    ) -> Result<(), BfExecErrorTy> {
        state.write.write_all(&buf[..*cursor])?;
        *cursor = 0;

        Ok(())
    }

    pub fn run<C: BfOptimizable, I: io::Read, O: io::Write>(
        &self,
        state: &mut state::BfState<C, I, O>,
    ) -> Result<(), BfExecError> {
        let mut idx = 0;

        let mut wbuf = [0; 32];
        let mut cursor = 0;
        let mut wloop = false;

        while idx < self.0.len() {
            match self.0[idx] {
                Executable::Zero => state.zero(),
                //         Executable::ZeroRange(by) => {
                //           todo!()
                //     }
                Executable::Inc(by) => state.inc(BfOptimizable::truncate_from(by)),
                Executable::Dec(by) => state.dec(BfOptimizable::truncate_from(by)),
                Executable::IncPtr(by) => state
                    .inc_ptr(by as usize)
                    .map_err(|source| BfExecError { source, idx })?,
                Executable::DecPtr(by) => state
                    .dec_ptr(by as usize)
                    .map_err(|source| BfExecError { source, idx })?,
                Executable::WLStart(to) => {
                    if state.jump_forward() {
                        idx = to as usize;
                    } else {
                        wloop = true;
                    }
                }
                Executable::WLEnd(to) => {
                    if state.jump_backward() {
                        idx = to as usize;
                    } else {
                        wloop = false;
                        Self::softflush(&mut wbuf, &mut cursor, state)
                            .map_err(|source| BfExecError { source, idx })?;
                    }
                }

                Executable::LStart(to) => {
                    if state.jump_forward() {
                        idx = to as usize;
                    }
                }
                Executable::LEnd(to) => {
                    if state.jump_backward() {
                        idx = to as usize;
                    }
                }
                Executable::Read => state.read().map_err(|source| BfExecError { source, idx })?,
                Executable::Write => if wloop {
                    Self::write(&mut wbuf, &mut cursor, state)
                } else {
                    state.write()
                }
                .map_err(|source| BfExecError { source, idx })?,
                Executable::Multiply(lut) => {
                    let dm = unsafe { self.1.get_unchecked(lut as usize) };

                    unsafe { state.mul(&dm.0, dm.1.iter().map(|ma| (ma.offset, ma.change))) }
                        .map_err(|source| BfExecError { source, idx })?;
                }
            }

            idx += 1;
        }

        Ok(())
    }
}
