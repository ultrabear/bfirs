//! Generic execution support

use crate::state::BfState;

/// Trait marking that an error type has a constructable out of instructions variant
pub trait HasOutOfInstructions<Ctx> {
    fn out_of_instructions(ctx: &Ctx) -> Self;
}

/// Implements a trivial common executor state for executors that only keep track of their currently
/// executing instruction
pub struct TrivialExecutorState {
    pub idx: usize,
}

/// Generic execution control interface
/// provide an initial state, a step function, and a running function
/// and Executor will supply a run and run_limited function
pub trait Executor<State, C, I, O, E> {
    /// Perform a single step through execution
    fn step(&self, exc_state: &mut State, bf_state: &mut BfState<C, I, O>) -> Result<(), E>;

    fn running(&self, exc_state: &State) -> bool;

    fn initial() -> State;

    /// Run once after running has returned false and no error occurred, or instruction limit reached
    #[inline(always)]
    fn finish(&self, exc_state: &mut State, state: &mut BfState<C, I, O>) -> Result<(), E> {
        _ = (exc_state, state);
        Ok(())
    }

    fn run(&self, state: &mut BfState<C, I, O>) -> Result<(), E> {
        let mut exc_state = Self::initial();

        while self.running(&exc_state) {
            self.step(&mut exc_state, state)?;
        }

        self.finish(&mut exc_state, state)
    }

    fn run_limited(&self, state: &mut BfState<C, I, O>, max_steps: u64) -> Result<(), E>
    where
        E: HasOutOfInstructions<State>,
    {
        let mut exc_state = Self::initial();

        match self.run_limited_from(&mut exc_state, state, max_steps) {
            Ok(()) => self.finish(&mut exc_state, state),
            Err(e) => Err(e),
        }
    }

    fn run_limited_from(
        &self,
        exc_state: &mut State,
        state: &mut BfState<C, I, O>,
        max_steps: u64,
    ) -> Result<(), E>
    where
        E: HasOutOfInstructions<State>,
    {
        let mut step = max_steps;

        while self.running(exc_state) {
            if step == 0 {
                self.finish(exc_state, state)?;
                return Err(E::out_of_instructions(&exc_state));
            }

            step = step.wrapping_sub(1);

            if let Err(e) = self.step(exc_state, state) {
                return Err(e);
            }
        }

        Ok(())
    }
}
