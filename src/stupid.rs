//! Stupid is a 1:1 "zero compile" bf interpreter
//! It only allocates to compute jump points, lazily during execution
//! This makes it suitable to interpret hundreds of gigabytes of bf, and not much else

use std::{collections::HashMap, io};

use either::Either;

use crate::{
    compiler::{BfCompError, BfOptimizable},
    executor::{Executor, HasOutOfInstructions},
    interpreter::{BfExecError, BfExecErrorTy},
    state::BfState,
};

fn lstart_jump(
    input: &[u8],
    mut cur: usize,
    cache: &mut HashMap<usize, usize>,
    iter: &mut Vec<usize>,
) -> Result<usize, BfCompError> {
    if let Some(&jump) = cache.get(&cur) {
        return Ok(jump);
    }

    iter.clear();

    while cur < input.len() {
        match input[cur] {
            b'[' => iter.push(cur),
            b']' => {
                if let Some(end) = iter.pop() {
                    cache.insert(cur, end);
                    cache.insert(end, cur);

                    if iter.is_empty() {
                        return Ok(cur);
                    }
                }
            }
            _ => {}
        }

        cur += 1;
    }

    Err(BfCompError::LoopCountMismatch)
}

fn lend_jump(
    input: &[u8],
    mut cur: usize,
    cache: &mut HashMap<usize, usize>,
    iter: &mut Vec<usize>,
) -> Result<usize, BfCompError> {
    if let Some(&jump) = cache.get(&cur) {
        return Ok(jump);
    }

    iter.clear();

    loop {
        match input[cur] {
            b']' => iter.push(cur),
            b'[' => {
                if let Some(end) = iter.pop() {
                    cache.insert(cur, end);
                    cache.insert(end, cur);

                    if iter.is_empty() {
                        return Ok(cur);
                    }
                }
            }
            _ => {}
        }

        if cur == 0 {
            break Err(BfCompError::LoopEndBeforeLoopStart);
        }

        cur -= 1;
    }
}

pub struct StupidExecutorState {
    cache: HashMap<usize, usize>,
    iter: Vec<usize>,
    idx: usize,
}

impl HasOutOfInstructions<StupidExecutorState> for Either<BfExecError, BfCompError> {
    fn out_of_instructions(ctx: &StupidExecutorState) -> Self {
        Self::Left(BfExecError {
            source: BfExecErrorTy::NotEnoughInstructions,
            idx: ctx.idx,
        })
    }
}

pub struct BfCode<'a>(pub &'a [u8]);

impl<'a, C, I, O> Executor<StupidExecutorState, C, I, O, Either<BfExecError, BfCompError>>
    for BfCode<'a>
where
    C: BfOptimizable,
    I: io::Read,
    O: io::Write,
{
    #[inline(always)]
    fn step(
        &self,
        exc_state: &mut StupidExecutorState,
        state: &mut BfState<C, I, O>,
    ) -> Result<(), Either<BfExecError, BfCompError>> {
        match self.0[exc_state.idx] {
            b'+' => state.inc(1.into()),
            b'-' => state.dec(1.into()),
            b'>' => {
                state
                    .inc_ptr(1)
                    .map_err(|s| BfExecError {
                        source: s,
                        idx: exc_state.idx,
                    })
                    .map_err(Either::Left)?;
            }
            b'<' => {
                state
                    .dec_ptr(1)
                    .map_err(|s| BfExecError {
                        source: s,
                        idx: exc_state.idx,
                    })
                    .map_err(Either::Left)?;
            }
            b'[' => {
                if state.jump_forward() {
                    exc_state.idx = lstart_jump(
                        self.0,
                        exc_state.idx,
                        &mut exc_state.cache,
                        &mut exc_state.iter,
                    )
                    .map_err(Either::Right)?;
                }
            }
            b']' => {
                if state.jump_backward() {
                    exc_state.idx = lend_jump(
                        self.0,
                        exc_state.idx,
                        &mut exc_state.cache,
                        &mut exc_state.iter,
                    )
                    .map_err(Either::Right)?;
                }
            }
            b',' => state
                .read()
                .map_err(|s| BfExecError {
                    source: s,
                    idx: exc_state.idx,
                })
                .map_err(Either::Left)?,

            b'.' => state
                .write()
                .map_err(|s| BfExecError {
                    source: s,
                    idx: exc_state.idx,
                })
                .map_err(Either::Left)?,
            _ => (),
        }

        exc_state.idx += 1;

        Ok(())
    }

    #[inline(always)]
    fn running(&self, exc_state: &StupidExecutorState) -> bool {
        exc_state.idx < self.0.len()
    }

    #[inline(always)]
    fn initial() -> StupidExecutorState {
        StupidExecutorState {
            cache: HashMap::new(),
            iter: Vec::new(),
            idx: 0,
        }
    }

    #[inline(always)]
    fn finish(
        &self,
        exc_state: &mut StupidExecutorState,
        state: &mut BfState<C, I, O>,
    ) -> Result<(), Either<BfExecError, BfCompError>> {
        state
            .write
            .flush()
            .map_err(BfExecErrorTy::from)
            .map_err(|s| BfExecError {
                source: s,
                idx: exc_state.idx,
            })
            .map_err(Either::Left)
    }
}
