use std::io;
use thiserror::Error;

use super::compiler::BfInstruc;

#[derive(Copy, Clone, Debug, Error)]
pub enum BfExecError {
	#[error("runtime overflowed its backing array")]
	Overflow,
	#[error("runtime underflowed its backing array")]
	Underflow,
	#[error("the pointer was already overflowed when the runtime started")]
	InitOverflow,
}

pub struct BrainFuckExecutor<T, I, O>
where
	O: io::Write,
	I: io::Read,
{
	pub stdout: O,
	pub stdin: I,
	pub data: Box<[T]>,
	pub ptr: usize,
}

impl BrainFuckExecutor<(), io::Stdin, io::Stdout> {
	pub fn new_stdio<T: Default>(array_len: usize) -> BrainFuckExecutor<T, io::Stdin, io::Stdout> {
		BrainFuckExecutor {
			stdout: io::stdout(),
			stdin: io::stdin(),
			data: std::iter::repeat(())
				.map(|_| T::default())
				.take(array_len)
				.collect(),
			ptr: 0,
		}
	}
}

macro_rules! impl_brainfuck_run {
	($T:ty) => {
		impl<I: io::Read, O: io::Write> BrainFuckExecutor<$T, I, O> {
			#[inline]
			unsafe fn cur_unchecked(&self) -> $T {
				// SAFETY: The caller has asserted that the current pointer is a valid index
				*self.data.get_unchecked(self.ptr)
			}

			#[inline]
			unsafe fn map_current(&mut self, func: impl FnOnce($T) -> $T) {
				// SAFETY: The caller has asserted that the current pointer is a valid index
				*self.data.get_unchecked_mut(self.ptr) = func(self.cur_unchecked());
			}

			#[inline]
			fn inc_ptr_by(&mut self, v: usize) -> Result<(), BfExecError> {
				self.ptr += v;
				if self.ptr >= self.data.len() {
					self.ptr -= v;
					return Err(BfExecError::Overflow);
				}
				Ok(())
			}

			#[inline]
			fn dec_ptr_by(&mut self, v: usize) -> Result<(), BfExecError> {
				self.ptr = self.ptr.checked_sub(v).ok_or(BfExecError::Underflow)?;
				Ok(())
			}

			#[inline]
			fn write(&mut self, v: u8) {
				if let Err(_) = self.stdout.write(&[v]) {}
			}

			#[inline]
			fn read(&mut self) -> u8 {
				let mut v = [0 as u8];
				if let Err(_) = self.stdin.read(&mut v) {
					v[0] = 0;
				}
				v[0]
			}

			pub fn run(&mut self, stream: &[BfInstruc<$T>]) -> Result<(), BfExecError> {
				use BfInstruc::*;

				let mut idx = 0usize;
				let len = stream.len();

				// SAFETY: check ptr bounds now to ensure they are valid before a _unchecked op is called without a ptr mutating op
				if self.ptr >= self.data.len() {
					return Err(BfExecError::InitOverflow);
				}

				// SAFETY: `ptr` bounds are checked by `ptr` mutating operations, so it will remain valid within this function
				while idx < len {
					unsafe {
						match stream[idx] {
							Zero => self.map_current(|_| 0),
							Inc => self.map_current(|c| c.wrapping_add(1)),
							Dec => self.map_current(|c| c.wrapping_sub(1)),
							IncPtr => self.inc_ptr_by(1)?,
							DecPtr => self.dec_ptr_by(1)?,
							Write => self.write(self.cur_unchecked() as u8),
							Read => {
								let v = self.read() as $T;
								self.map_current(|_| v);
							}
							LStart(end) => {
								if self.cur_unchecked() == 0 {
									idx = end as usize;
								}
							}
							LEnd(start) => {
								if self.cur_unchecked() != 0 {
									idx = start as usize;
								}
							}
							IncBy(val) => self.map_current(|c| c.wrapping_add(val)),
							DecBy(val) => self.map_current(|c| c.wrapping_sub(val)),
							IncPtrBy(val) => self.inc_ptr_by(val.get() as usize)?,
							DecPtrBy(val) => self.dec_ptr_by(val.get() as usize)?,
						}

						idx += 1;
					}
				}

				Ok(())
			}
		}
	};
}

impl_brainfuck_run!(u8);
impl_brainfuck_run!(u16);
impl_brainfuck_run!(u32);
