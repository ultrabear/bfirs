//! Stupid is a 1:1 "zero compile" bf interpreter
//! It only allocates to compute jump points, lazily during execution
//! This makes it suitable to interpret hundreds of gigabytes of bf, and not much else

use std::{collections::HashMap, io};

use either::Either;

use crate::{
    compiler::{BfCompError, BfOptimizable},
    interpreter::{BfExecError, BfExecErrorTy},
};

pub struct BfState<T, I, O>
where
    T: BfOptimizable,
    I: io::Read,
    O: io::Write,
{
    pub ptr: usize,
    pub data: Box<[T]>,
    pub read: I,
    pub write: O,
}

impl<T: BfOptimizable, I: io::Read, O: io::Write> BfState<T, I, O> {
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
        let _ = self.write.write(&[v])?;
        Ok(())
    }

    fn read(&mut self) -> Result<u8, BfExecErrorTy> {
        // flush so the end user always gets prompts
        self.write.flush()?;

        let mut v = [0];
        let _ = self.read.read(&mut v)?;
        Ok(v[0])
    }

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

    pub fn run(&mut self, input: &[u8]) -> Result<(), Either<BfExecError, BfCompError>> {
        if self.ptr >= self.data.len() {
            return Err(Either::Left(BfExecError {
                source: BfExecErrorTy::InitOverflow,
                idx: 0,
            }));
        }

        let mut cache = HashMap::<usize, usize>::new();
        let mut iter = Vec::<usize>::new();

        let mut idx = 0;

        while idx < input.len() {
            match input[idx] {
                b'+' => unsafe { self.set(self.get().wrapping_add(1.into())) },
                b'-' => unsafe { self.set(self.get().wrapping_sub(1.into())) },
                b'>' => {
                    self.inc_ptr_by(1)
                        .map_err(|s| BfExecError { source: s, idx })
                        .map_err(Either::Left)?;
                }
                b'<' => {
                    self.dec_ptr_by(1)
                        .map_err(|s| BfExecError { source: s, idx })
                        .map_err(Either::Left)?;
                }
                b'[' => {
                    if unsafe { self.get() } == T::ZERO {
                        idx = Self::lstart_jump(input, idx, &mut cache, &mut iter)
                            .map_err(Either::Right)?;
                    }
                }
                b']' => {
                    if unsafe { self.get() } != T::ZERO {
                        idx = Self::lend_jump(input, idx, &mut cache, &mut iter)
                            .map_err(Either::Right)?;
                    }
                }
                b',' => {
                    let v = T::from(
                        self.read()
                            .map_err(|s| BfExecError { source: s, idx })
                            .map_err(Either::Left)?,
                    );

                    unsafe { self.set(v) };
                }
                b'.' => self
                    .write(unsafe { self.get().truncate_u8() })
                    .map_err(|s| BfExecError { source: s, idx })
                    .map_err(Either::Left)?,
                _ => (),
            }

            idx += 1;
        }

        self.write
            .flush()
            .map_err(BfExecErrorTy::from)
            .map_err(|s| BfExecError { source: s, idx })
            .map_err(Either::Left)?;

        Ok(())
    }
}
