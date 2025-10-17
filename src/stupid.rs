//! Stupid is a 1:1 "zero compile" bf interpreter
//! It only allocates to compute jump points, lazily during execution
//! This makes it suitable to interpret hundreds of gigabytes of bf, and not much else

use std::{collections::HashMap, io};

use either::Either;

use crate::{
    compiler::{BfCompError, BfOptimizable},
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

pub fn interpret<C: BfOptimizable, I: io::Read, O: io::Write>(
    input: &[u8],
    state: &mut BfState<C, I, O>,
) -> Result<(), Either<BfExecError, BfCompError>> {
    if let Err(e) = state.validate_init_ptr() {
        return Err(Either::Left(BfExecError { source: e, idx: 0 }));
    }

    let mut cache = HashMap::<usize, usize>::new();
    let mut iter = Vec::<usize>::new();

    let mut idx = 0;

    while idx < input.len() {
        match input[idx] {
            b'+' => unsafe { state.inc(1.into()) },
            b'-' => unsafe { state.dec(1.into()) },
            b'>' => {
                state
                    .inc_ptr(1)
                    .map_err(|s| BfExecError { source: s, idx })
                    .map_err(Either::Left)?;
            }
            b'<' => {
                state
                    .dec_ptr(1)
                    .map_err(|s| BfExecError { source: s, idx })
                    .map_err(Either::Left)?;
            }
            b'[' => {
                if unsafe { state.jump_forward() } {
                    idx = lstart_jump(input, idx, &mut cache, &mut iter).map_err(Either::Right)?;
                }
            }
            b']' => {
                if unsafe { state.jump_backward() } {
                    idx = lend_jump(input, idx, &mut cache, &mut iter).map_err(Either::Right)?;
                }
            }
            b',' => unsafe {
                state
                    .read()
                    .map_err(|s| BfExecError { source: s, idx })
                    .map_err(Either::Left)?;
            },
            b'.' => unsafe {
                state
                    .write()
                    .map_err(|s| BfExecError { source: s, idx })
                    .map_err(Either::Left)?
            },
            _ => (),
        }

        idx += 1;
    }

    state
        .write
        .flush()
        .map_err(BfExecErrorTy::from)
        .map_err(|s| BfExecError { source: s, idx })
        .map_err(Either::Left)?;

    Ok(())
}
