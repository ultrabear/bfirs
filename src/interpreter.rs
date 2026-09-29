use core::fmt;
use std::io;
use thiserror::Error;

use crate::{
    compiler::BfOptimizable,
    executor::{Executor, HasOutOfInstructions, TrivialExecutorState},
    state::BfState,
};

use super::compiler::BfInstruc;

#[derive(Debug, Error)]
pub struct BfExecError {
    pub source: BfExecErrorTy,
    pub idx: usize,
}

impl HasOutOfInstructions<TrivialExecutorState> for BfExecError {

    #[inline(always)]
    fn out_of_instructions(ctx: &TrivialExecutorState) -> Self {
        Self {
            source: BfExecErrorTy::NotEnoughInstructions,
            idx: ctx.idx,
        }
    }
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

#[derive(Debug, Error)]
pub struct Overflow;

impl fmt::Display for Overflow {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "overflow while attempting to add instruction limit")
    }
}

pub struct StandardExecutor<'a, C>(pub &'a [BfInstruc<C>]);

impl<C, I, O> Executor<TrivialExecutorState, C, I, O, BfExecError> for StandardExecutor<'_, C>
where
    C: BfOptimizable,
    I: io::Read,
    O: io::Write,
{
    #[inline(always)]
    fn step(
        &self,
        exc_state: &mut TrivialExecutorState,
        state: &mut BfState<C, I, O>,
    ) -> Result<(), BfExecError> {
        use BfInstruc::*;

        {
            match self.0[exc_state.idx] {
                Zero => {
                    state.zero();
                    Ok(())
                }
                Inc => {
                    state.inc(1.into());
                    Ok(())
                }
                Dec => {
                    state.dec(1.into());
                    Ok(())
                }
                IncPtr => state.inc_ptr(1),
                DecPtr => state.dec_ptr(1),
                Write => state.write(),
                Read => state.read(),
                LStart(end) => {
                    if state.jump_forward() {
                        exc_state.idx = end as usize;
                    }
                    Ok(())
                }
                LEnd(start) => {
                    if state.jump_backward() {
                        exc_state.idx = start as usize;
                    }
                    Ok(())
                }
                IncBy(val) => {
                    state.inc(val);
                    Ok(())
                }
                DecBy(val) => {
                    state.dec(val);
                    Ok(())
                }
                IncPtrBy(val) => state.inc_ptr(val.get() as usize),
                DecPtrBy(val) => state.dec_ptr(val.get() as usize),
            }
        }
        .map_err(|source| BfExecError {
            source,
            idx: exc_state.idx,
        })?;

        exc_state.idx += 1;

        Ok(())
    }

    #[inline(always)]
    fn running(&self, exc_state: &TrivialExecutorState) -> bool {
        exc_state.idx < self.0.len()
    }

    #[inline(always)]
    fn initial() -> TrivialExecutorState {
        TrivialExecutorState { idx: 0 }
    }
}

#[test]
fn test_exec_env() {
    use super::compiler::BfInstructionStream;

    let parse_bf =
        |code: &str| BfInstructionStream::optimized_from_text(code.bytes(), None).unwrap();

    let run_code = |x: &str| {
        let mut state = BfState::new(
            0,
            vec![0u8; 30_000].into_boxed_slice(),
            io::stdin(),
            io::stdout(),
        )
        .map_err(|_| ())
        .unwrap();

        StandardExecutor(&parse_bf(x)).run(&mut state).unwrap();
    };

    let expect_output = |code: &str, expect: &str| {
        let mut outv = Vec::new();

        let mut state = BfState::new(
            0,
            vec![0u8; 30_000].into_boxed_slice(),
            io::empty(),
            &mut outv,
        )
        .map_err(|_| ())
        .unwrap();

        StandardExecutor(&parse_bf(code)).run(&mut state).unwrap();

        if outv != expect.as_bytes() {
            panic!("Expected {}, got instead {:?}", expect, outv);
        }
    };

    macro_rules! expect_error {
        ($s:expr, $err:pat, $rep:expr) => {
            let mut state = BfState::new(
                0,
                vec![0u8; 30_000].into_boxed_slice(),
                io::stdin(),
                io::stdout(),
            )
            .map_err(|_| ())
            .unwrap();

            let exc = StandardExecutor(&parse_bf($s));

            match exc.run_limited(&mut state, 1_000_000) {
                Ok(_) => panic!("Got Ok(()) value, expected {:?}", $rep),
                Err(err) => match err {
                    BfExecError { source: $err, .. } => (),
                    e => panic!("Got {:?} value, expected {:?}", e, $rep),
                },
            };

            drop(state);
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
