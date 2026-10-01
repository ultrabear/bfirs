//! An intermediate DAG representation for a BF programs optimization stage

use std::{collections::HashMap, hash::Hash, io, ops::Range};

use crate::{
    compiler::{BfCompError, BfOptimizable},
    executor::{Executor, HasOutOfInstructions},
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

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
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
    fn terminates_no_read(&self) -> bool {
        match self {
            Self::Loop(_) | Self::WriteLoop(_) => false,
            Self::If(c) => c.iter().all(|n| n.terminates_no_read()),
            Self::Read => false,
            ITree::Zero
            | ITree::Mul(_, _)
            | ITree::Inc(_)
            | ITree::Dec(_)
            | ITree::IncPtr(_)
            | ITree::DecPtr(_)
            | ITree::Write => true,
        }
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
            && this.iter().all(|c| c.terminates_no_read())
            && this.iter().any(|c| matches!(c, Self::Write))
    }

    fn as_multiply(this: &[Self]) -> Option<(Range<isize>, Vec<MulArg>)> {
        const Z_OFFSET: usize = 128;

        let mut minivm = [0i64; Z_OFFSET * 2];
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

    fn synth_inner(
        this: &[Self],
        stream: &mut Vec<Executable>,
        cache: &mut CacheBuilder<DistinctMultiply>,
        sim_cache: &mut CacheBuilder<MulArg>,
    ) {
        for node in this {
            match node {
                ITree::Zero => stream.push(Executable::Zero),
                ITree::Mul(range, mul_args) => {
                    if mul_args.len() == 1 {
                        let i = sim_cache.insert(mul_args[0].clone());
                        stream.push(Executable::SingleMultiply(i));
                    } else {
                        let i = cache.insert(DistinctMultiply {
                            bound: range.clone(),
                            muls: mul_args.clone(),
                        });

                        stream.push(Executable::Multiply(i));
                    }
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
                    Self::synth_inner(&itrees, stream, cache, sim_cache);

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
                    Self::synth_inner(&itrees, stream, cache, sim_cache);
                    let e_idx = stream.len();
                    stream.push(Executable::WLEnd(s_idx as u32));

                    stream[s_idx] = Executable::WLStart(e_idx as u32);
                }
                ITree::If(itrees) => {
                    let s_idx = stream.len();
                    stream.push(Executable::LStart(0));
                    Self::synth_inner(&itrees, stream, cache, sim_cache);
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
        let mut cache = CacheBuilder::default();
        let mut sim_cache = CacheBuilder::default();

        let mut stream = vec![];

        Self::synth_inner(this, &mut stream, &mut cache, &mut sim_cache);

        InterpreterStream(stream, cache.output(), sim_cache.output())
    }
}

pub fn find_if_conditions(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(children) = node {
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
        } else if let ITree::If(children) = node {
            rewrite_multiply(children);
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
        } else if let ITree::If(children) = node {
            rewrite_write_loops(children);
        }
    }
}

/// Applies standard optimization pipeline in order
pub fn standard_pipeline(tree: &mut [ITree]) {
    find_if_conditions(tree);
    rewrite_multiply(tree);
    rewrite_write_loops(tree);
}

#[derive(Debug, Hash, Eq, PartialEq, Clone)]
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
    SingleMultiply(u32),
    //    ZeroRange(u32),
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct DistinctMultiply {
    bound: Range<isize>,
    muls: Vec<MulArg>,
}

pub struct CacheBuilder<T>(Vec<T>, HashMap<T, u32>);

impl<T> Default for CacheBuilder<T> {
    fn default() -> Self {
        Self(vec![], HashMap::default())
    }
}

impl<T: Hash + Eq + Clone> CacheBuilder<T> {
    fn insert(&mut self, dm: T) -> u32 {
        if let Some(v) = self.1.get(&dm) {
            *v
        } else {
            let idx = self.0.len() as u32;

            self.1.insert(dm.clone(), idx);
            self.0.push(dm);

            idx
        }
    }

    fn output(self) -> Vec<T> {
        self.0
    }
}

#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub struct InterpreterStream(Vec<Executable>, Vec<DistinctMultiply>, Vec<MulArg>);

pub struct OExecutorState {
    idx: usize,
    buf: [u8; 32],
    cursor: usize,
    wloop: bool,
}

impl HasOutOfInstructions<OExecutorState> for BfExecError {
    fn out_of_instructions(ctx: &OExecutorState) -> Self {
        BfExecError {
            source: BfExecErrorTy::NotEnoughInstructions,
            idx: ctx.idx,
        }
    }
}

impl OExecutorState {
    fn softwrite<C: BfOptimizable, I: io::Read, O: io::Write>(
        &mut self,
        state: &mut state::BfState<C, I, O>,
    ) -> Result<(), BfExecErrorTy> {
        if self.cursor == self.buf.len() {
            state.write.write_all(&self.buf)?;
            self.cursor = 0;
        }

        self.buf[self.cursor] = state.get().truncate_u8();
        self.cursor += 1;

        Ok(())
    }

    fn softflush<C: BfOptimizable, I: io::Read, O: io::Write>(
        &mut self,
        state: &mut state::BfState<C, I, O>,
    ) -> Result<(), BfExecErrorTy> {
        state.write.write_all(&self.buf[..self.cursor])?;
        self.cursor = 0;

        Ok(())
    }
}

impl<C, I, O> Executor<OExecutorState, C, I, O, BfExecError> for InterpreterStream
where
    C: BfOptimizable,
    I: io::Read,
    O: io::Write,
{
    #[inline(always)]
    fn initial() -> OExecutorState {
        OExecutorState {
            idx: 0,
            buf: [0; 32],
            cursor: 0,
            wloop: false,
        }
    }

    #[inline(always)]
    fn running(&self, exc_state: &OExecutorState) -> bool {
        exc_state.idx < self.0.len()
    }

    fn finish(
        &self,
        exc_state: &mut OExecutorState,
        state: &mut state::BfState<C, I, O>,
    ) -> Result<(), BfExecError> {
        if exc_state.wloop {
            exc_state.softflush(state).map_err(|source| BfExecError {
                source,
                idx: exc_state.idx,
            })
        } else {
            Ok(())
        }
    }

    #[inline(always)]
    fn step(
        &self,
        exc_state: &mut OExecutorState,
        state: &mut state::BfState<C, I, O>,
    ) -> Result<(), BfExecError> {
        match self.0[exc_state.idx] {
            Executable::Zero => state.zero(),
            //         Executable::ZeroRange(by) => {
            //           todo!()
            //     }
            Executable::Inc(by) => state.inc(BfOptimizable::truncate_from(by)),
            Executable::Dec(by) => state.dec(BfOptimizable::truncate_from(by)),
            Executable::IncPtr(by) => state.inc_ptr(by as usize).map_err(|source| BfExecError {
                source,
                idx: exc_state.idx,
            })?,
            Executable::DecPtr(by) => state.dec_ptr(by as usize).map_err(|source| BfExecError {
                source,
                idx: exc_state.idx,
            })?,
            Executable::WLStart(to) => {
                if state.jump_forward() {
                    exc_state.idx = to as usize;
                } else {
                    exc_state.wloop = true;
                }
            }
            Executable::WLEnd(to) => {
                if state.jump_backward() {
                    exc_state.idx = to as usize;
                } else {
                    exc_state.wloop = false;
                    exc_state.softflush(state).map_err(|source| BfExecError {
                        source,
                        idx: exc_state.idx,
                    })?;
                }
            }

            Executable::LStart(to) => {
                if state.jump_forward() {
                    exc_state.idx = to as usize;
                }
            }
            Executable::LEnd(to) => {
                if state.jump_backward() {
                    exc_state.idx = to as usize;
                }
            }
            Executable::Read => state.read().map_err(|source| BfExecError {
                source,
                idx: exc_state.idx,
            })?,
            Executable::Write => if exc_state.wloop {
                exc_state.softwrite(state)
            } else {
                state.write()
            }
            .map_err(|source| BfExecError {
                source,
                idx: exc_state.idx,
            })?,
            Executable::Multiply(lut) => {
                let dm = unsafe { self.1.get_unchecked(lut as usize) };

                unsafe { state.mul(&dm.bound, dm.muls.iter().map(|ma| (ma.offset, ma.change))) }
                    .map_err(|source| BfExecError {
                        source,
                        idx: exc_state.idx,
                    })?;
            }
            Executable::SingleMultiply(lut) => {
                let single = unsafe { self.2.get_unchecked(lut as usize) };

                state
                    .single_mul(single.offset, single.change)
                    .map_err(|source| BfExecError {
                        source,
                        idx: exc_state.idx,
                    })?;
            }
        }

        exc_state.idx += 1;

        Ok(())
    }
}

#[test]
fn valid_mul() {
    use rayon::iter::{IntoParallelIterator, ParallelIterator};

    [false, true].into_par_iter().for_each(|inc| {
        (u8::MIN..=u8::MAX).into_par_iter().for_each(|a| {
            let mut state =
                state::BfState::new(0, Box::new([0, 0]), io::empty(), io::sink()).unwrap();

            let iter = vec![ITree::Loop(vec![
                ITree::Dec(1),
                ITree::IncPtr(1),
                if inc {
                    ITree::Inc(a as u32)
                } else {
                    ITree::Dec(a as u32)
                },
                ITree::DecPtr(1),
            ])];

            let mut dag = iter.clone();

            rewrite_multiply(&mut dag);

            if a != 0 {
                assert_eq!(
                    dag,
                    vec![ITree::Mul(
                        0..1,
                        vec![MulArg {
                            offset: 1,
                            change: if inc { a as i64 } else { -(a as i64) },
                        },],
                    )]
                );
            } else {
                assert_eq!(dag, vec![ITree::Mul(0..1, vec![])]);
            }

            let iter = ITree::synthesize(&iter);
            let dag = ITree::synthesize(&dag);

            for b in u8::MIN..=u8::MAX {
                for c in u8::MIN..=u8::MAX {
                    unsafe { state.set_ptr(0) };
                    state.cells_mut()[0] = b;
                    state.cells_mut()[1] = c;

                    let mut state_iter = state.clone();
                    iter.run(&mut state_iter).unwrap();

                    let mut state_dag = state.clone();
                    dag.run(&mut state_dag).unwrap();

                    assert_eq!(state_dag.comparable(), state_iter.comparable());
                }
            }
        });
    });
}

#[test]
fn mul_bounds_check() {
    let mut tree = Token::to_tree(&Token::parse(b"[-<+>]")).unwrap();

    standard_pipeline(&mut tree);

    let runnable = ITree::synthesize(&tree);

    let mut state = state::BfState::new(0, Box::new([0u8]), io::empty(), io::sink()).unwrap();

    runnable
        .run(&mut state)
        .expect("OOB should not hit when branch is not taken");
}
