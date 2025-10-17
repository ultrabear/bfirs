//! Base types with common implementations

use std::io;

use crate::{compiler::BfOptimizable, interpreter::BfExecErrorTy};

/// The state of a bf programs memory region
pub struct BfState<C, I, O> {
    pub ptr: usize,
    pub cells: Box<[C]>,
    pub read: I,
    pub write: O,
}

impl<C: BfOptimizable, I: io::Read, O: io::Write> BfState<C, I, O> {
    #[inline(always)]
    pub fn validate_init_ptr(&self) -> Result<(), BfExecErrorTy> {
        if self.ptr < self.cells.len() {
            Ok(())
        } else {
            Err(BfExecErrorTy::InitOverflow)
        }
    }

    #[inline(always)]
    pub unsafe fn get(&self) -> C {
        *self.cells.get_unchecked(self.ptr)
    }

    #[inline(always)]
    pub unsafe fn set(&mut self, data: C) {
        *self.cells.get_unchecked_mut(self.ptr) = data;
    }

    #[inline(always)]
    pub unsafe fn map<F: FnOnce(C) -> C>(&mut self, transform: F) {
        self.set(transform(self.get()))
    }

    #[inline(always)]
    pub unsafe fn zero(&mut self) {
        self.set(C::ZERO);
    }

    #[inline(always)]
    pub unsafe fn write(&mut self) -> Result<(), BfExecErrorTy> {
        let cell = self.get().truncate_u8();

        self.write.write(&[cell])?;

        Ok(())
    }

    #[inline(always)]
    pub unsafe fn read(&mut self) -> Result<(), BfExecErrorTy> {
        self.write.flush()?;

        let mut out = [0u8; 1];

        self.read.read(&mut out)?;

        self.set(out[0].into());
        Ok(())
    }

    #[inline(always)]
    pub unsafe fn inc(&mut self, by: C) {
        self.map(|c| c.wrapping_add(by))
    }

    #[inline(always)]
    pub unsafe fn dec(&mut self, by: C) {
        self.map(|c| c.wrapping_sub(by))
    }

    #[inline(always)]
    pub fn inc_ptr(&mut self, by: usize) -> Result<(), BfExecErrorTy> {
        match self.ptr.checked_add(by) {
            Some(incremented) => {
                if incremented < self.cells.len() {
                    self.ptr = incremented;
                    Ok(())
                } else {
                    Err(BfExecErrorTy::Overflow)
                }
            }
            None => Err(BfExecErrorTy::Overflow),
        }
    }

    #[inline(always)]
    pub fn dec_ptr(&mut self, by: usize) -> Result<(), BfExecErrorTy> {
        match self.ptr.checked_sub(by) {
            Some(decrement) => {
                self.ptr = decrement;
                Ok(())
            }
            None => Err(BfExecErrorTy::Underflow),
        }
    }

    #[inline(always)]
    pub unsafe fn jump_forward(&self) -> bool {
        self.get() == C::ZERO
    }

    #[inline(always)]
    pub unsafe fn jump_backward(&self) -> bool {
        self.get() != C::ZERO
    }
}
