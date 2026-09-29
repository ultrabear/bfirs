//! Base types with common implementations

use core::fmt;
use std::{io, ops::Range};

use crate::{compiler::BfOptimizable, interpreter::BfExecErrorTy};

/// The state of a bf programs memory region
#[derive(Clone)]
pub struct BfState<C, I, O> {
    ptr: usize,
    cells: Box<[C]>,
    pub read: I,
    pub write: O,
}

impl<C: fmt::Debug, I, O> fmt::Debug for BfState<C, I, O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BfState")
            .field("ptr", &self.ptr)
            .field("cells", &self.cells)
            .finish_non_exhaustive()
    }
}

impl<C, I, O> BfState<C, I, O> {
    /// Attempts to construct a new checked BfState
    pub fn new(
        ptr: usize,
        cells: Box<[C]>,
        read: I,
        write: O,
    ) -> Result<Self, (usize, Box<[C]>, I, O)> {
        if ptr < cells.len() {
            Ok(Self {
                ptr,
                cells,
                read,
                write,
            })
        } else {
            Err((ptr, cells, read, write))
        }
    }

    pub fn ptr(&self) -> usize {
        self.ptr
    }

    pub unsafe fn set_ptr(&mut self, ptr: usize) {
        self.ptr = ptr;
    }

    pub fn cells(&self) -> &[C] {
        &self.cells
    }

    #[cfg(test)]
    pub fn cells_mut(&mut self) -> &mut [C] {
        &mut self.cells
    }

    #[cfg(test)]
    pub fn comparable(&self) -> (usize, &[C]) {
        (self.ptr(), self.cells())
    }
}

impl<C: BfOptimizable, I: io::Read, O: io::Write> BfState<C, I, O> {
    #[inline(always)]
    pub fn get(&self) -> C {
        // SAFETY: ptr<self.cells.len() is always upheld
        unsafe { *self.cells.get_unchecked(self.ptr) }
    }

    #[inline(always)]
    pub fn set(&mut self, data: C) {
        // SAFETY: ptr<self.cells.len() is always upheld
        unsafe {
            *self.cells.get_unchecked_mut(self.ptr) = data;
        }
    }

    #[inline(always)]
    pub fn map<F: FnOnce(C) -> C>(&mut self, transform: F) {
        self.set(transform(self.get()))
    }

    #[inline(always)]
    pub fn zero(&mut self) {
        self.set(C::ZERO);
    }

    #[inline(always)]
    pub fn write(&mut self) -> Result<(), BfExecErrorTy> {
        let cell = self.get().truncate_u8();

        self.write.write_all(&[cell])?;

        Ok(())
    }

    #[inline(always)]
    pub fn read(&mut self) -> Result<(), BfExecErrorTy> {
        self.write.flush()?;

        let mut out = [0u8; 1];

        self.read.read(&mut out)?;

        self.set(out[0].into());
        Ok(())
    }

    #[inline(always)]
    pub fn inc(&mut self, by: C) {
        self.map(|c| c.wrapping_add(by))
    }

    #[inline(always)]
    pub fn dec(&mut self, by: C) {
        self.map(|c| c.wrapping_sub(by))
    }

    #[inline(always)]
    pub fn inc_ptr(&mut self, by: usize) -> Result<(), BfExecErrorTy> {
        match self.ptr.checked_add(by) {
            Some(incremented) => {
                if incremented < self.cells.len() {
                    // SAFETY: we checked the incremented amount fits in the cells
                    unsafe { self.set_ptr(incremented) };
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
                // SAFETY: we checked the decremented amount does not underflow
                unsafe { self.set_ptr(decrement) };
                Ok(())
            }
            None => Err(BfExecErrorTy::Underflow),
        }
    }

    #[inline(always)]
    pub fn jump_forward(&self) -> bool {
        self.get() == C::ZERO
    }

    #[inline(always)]
    pub fn jump_backward(&self) -> bool {
        self.get() != C::ZERO
    }

    /// # Safety
    /// Bounds must hold the minimum and maximum index that will be visited by all operators:
    /// - for operators (1isize, _), bounds must be at least 0..1
    /// - for operators (-1isize, _), bounds must be at least -1..0
    #[inline(always)]
    pub unsafe fn mul(
        &mut self,
        bounds: &Range<isize>,
        operators: impl Iterator<Item = (isize, i64)>,
    ) -> Result<(), BfExecErrorTy> {
        if self.jump_forward() {
            return Ok(());
        }

        let Some(lower) = self.ptr().checked_add_signed(bounds.start) else {
            return Err(BfExecErrorTy::Underflow);
        };
        let Some(upper) = self.ptr().checked_add_signed(bounds.end) else {
            return Err(BfExecErrorTy::Underflow);
        };
        let true = lower < self.cells.len() else {
            return Err(BfExecErrorTy::Overflow);
        };
        let true = upper < self.cells.len() else {
            return Err(BfExecErrorTy::Overflow);
        };

        let by = i64::from(self.get().into());
        self.zero();

        for (offset, diff) in operators {
            let idx = (self.ptr as isize).unchecked_add(offset) as usize;

            self.cells.get_unchecked_mut(idx).add(by.wrapping_mul(diff));
        }

        Ok(())
    }
}
