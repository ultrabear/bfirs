#[repr(u8)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum BfInstruc<CellSize: Clone> {
	Zero,
	Inc,
	Dec,
	IncPtr,
	DecPtr,
	Write,
	Read,
	LStart(u32),
	LEnd(u32),
	IncBy(CellSize),
	DecBy(CellSize),
	IncPtrBy(u32),
	DecPtrBy(u32),
}

impl<T: Clone> TryFrom<u8> for BfInstruc<T> {
	type Error = u8;

	fn try_from(value: u8) -> Result<Self, Self::Error> {
		use BfInstruc::*;

		Ok(match value {
			b'+' => Inc,
			b'-' => Dec,
			b'>' => IncPtr,
			b'<' => DecPtr,
			b'.' => Write,
			b',' => Read,
			// Jump points must be computed later by the full stream parser
			b'[' => LStart(0),
			b']' => LEnd(0),
			_ => {
				return Err(value);
			}
		})
	}
}

impl<T: Clone> BfInstruc<T> {
	fn as_multi_with(&self, v: u32) -> Option<BfInstruc<T>>
	where
		T: TryFrom<u32>,
    <T as TryFrom<u32>>::Error: std::fmt::Debug,
	{
		use BfInstruc::*;

		Some(match self {
			Inc => IncBy(v.try_into().unwrap()),
			Dec => DecBy(v.try_into().unwrap()),
			IncPtr => IncPtrBy(v),
			DecPtr => DecPtrBy(v),
			_ => None?,
		})
	}
}

use std::io;

pub struct BrainFuckExecutor<CellSize: Clone, I, O>
where
	O: io::Write,
	I: io::Read,
{
	pub stdout: O,
	pub stdin: I,
	pub data: Box<[CellSize]>,
	pub ptr: usize,
}

macro_rules! impl_brainfuck_run {
	($T:ty) => {
		impl<I: io::Read, O: io::Write> BrainFuckExecutor<$T, I, O> {
			unsafe fn cur_unchecked(&self) -> $T {
				// SAFETY: The caller has asserted that the current pointer is a valid index
				{
					*self.data.get_unchecked(self.ptr)
				}
			}

			unsafe fn map_current(&mut self, func: impl FnOnce($T) -> $T) {
				{
					*self.data.get_unchecked_mut(self.ptr) = func(self.cur_unchecked())
				};
			}

			fn inc_ptr_by(&mut self, v: usize) {
				self.ptr += v;
				if self.ptr >= self.data.len() {
					panic!("RUNTIME MEMORY OVERFLOW") // TODO less shit error handling
				}
			}

			fn dec_ptr_by(&mut self, v: usize) {
				self.ptr -= v;
				if self.ptr >= self.data.len() {
					panic!("RUNTIME MEMORY UNDERFLOW") // TODO less shit error handling
				}
			}

			#[inline]
			fn write(&mut self, v: u8) {
				if let Err(_) = self.stdout.write(&[v]) {}
			}

			fn read(&mut self) -> u8 {
				let mut v = [0 as u8];
				if let Err(_) = self.stdin.read(&mut v) {
					v[0] = 0;
				}
				v[0]
			}

			pub fn run(&mut self, stream: &[BfInstruc<$T>]) {
				use BfInstruc::*;

				let mut idx = 0usize;
				let len = stream.len();

				// SAFETY: check ptr bounds now to ensure they are valid before a _unchecked op is called without a ptr mutating op
				if self.ptr >= self.data.len() {
					panic!("PTR POINTS TO INVALID MEMORY")
				}

				// SAFETY: `ptr` bounds are checked by `ptr` mutating operations, so it will remain valid within this function
				unsafe {
					while idx < len {
						match stream[idx] {
							Zero => self.map_current(|_| 0),
							Inc => self.map_current(|c| c.wrapping_add(1)),
							Dec => self.map_current(|c| c.wrapping_sub(1)),
							IncPtr => self.inc_ptr_by(1),
							DecPtr => self.dec_ptr_by(1),
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
							IncPtrBy(val) => self.inc_ptr_by(val as usize),
							DecPtrBy(val) => self.dec_ptr_by(val as usize),
						}

						idx += 1;
					}
				}
			}
		}
	};
}

impl_brainfuck_run!(u8);
impl_brainfuck_run!(u16);
impl_brainfuck_run!(u32);
impl_brainfuck_run!(u64);

fn bf_to_stream<CellSize: Clone>(v: impl Iterator<Item = u8>) -> Vec<BfInstruc<CellSize>> {
	v.filter_map(|byte| BfInstruc::try_from(byte).ok())
		.collect()
}

fn is_optimizable<T: Clone>(instr: &BfInstruc<T>) -> bool {
	use BfInstruc::*;
	match instr {
		Inc | Dec | IncPtr | DecPtr => true,
		_ => false,
	}
}

// without this inline attr it fails to inline this function into the mainloop, preventing a considerable speedup
#[inline]
fn group_common_bf<T: Copy + Clone + Eq + Into<u32> + TryFrom<u32>>(
	mut stream: Vec<BfInstruc<T>>,
) -> Vec<BfInstruc<T>>
where
	<T as TryFrom<u32>>::Error: std::fmt::Debug,
{
	let mut newlen = 0usize;

	let mut i = 0usize;
	while i < stream.len() {
		if is_optimizable(&stream[i]) {
			let mut ctr = 1;

			while (i + 1 < stream.len()) && (stream[i] == stream[i + 1]) {
				i += 1;
				ctr += 1;
			}

			if ctr == 1 {
				stream[newlen] = stream[i];
				newlen += 1;
			} else {
				stream[newlen] = stream[i].as_multi_with(ctr).unwrap();
				newlen += 1
			}
		} else {
			stream[newlen] = stream[i];
			newlen += 1;
		}

		i += 1;
	}

	stream.truncate(newlen);
	stream
}

fn insert_bf_jump_points<CellSize: Clone>(stream: &mut [BfInstruc<CellSize>]) {
	let mut stack = Vec::<usize>::new();

	for idx in 0..stream.len() {
		match stream[idx] {
			BfInstruc::LStart(_) => {
				stack.push(idx);
			}
			BfInstruc::LEnd(_) => {
				if let Some(v) = stack.pop() {
					stream[v] = BfInstruc::LStart(idx as u32);
					stream[idx] = BfInstruc::LEnd(v as u32);
				} else {
					panic!("ERROR: Loop end defined before loop start")
				}
			}
			_ => {}
		}
	}
}

use std::fs::File;
use std::io::prelude::*;

fn get_bf_stream_from_args<T: Clone>() -> Vec<BfInstruc<T>> {
	//let args = std::env::args().skip(1).collect::<String>();
	//let code = args.as_bytes().iter().copied();

	let mut code_f = File::open("code.bf").unwrap();

	let mut falloc = Vec::new();

	code_f.read_to_end(&mut falloc).unwrap();

	bf_to_stream(falloc.into_iter())
}

fn main() {
	let code = get_bf_stream_from_args();

	let incptr_count: u32 = code.iter().fold(0, |accu, x| {
		if let BfInstruc::IncPtr = x {
			accu + 1
		} else {
			accu
		}
	});

	let mut code = group_common_bf(code);

	insert_bf_jump_points(&mut code);

	let backing = std::iter::repeat(0u8)
		.take(incptr_count.max(30_000) as usize)
		.collect();

	let mut execenv = BrainFuckExecutor {
		stdout: io::stdout(),
		stdin: io::stdin(),
		ptr: 0,
		data: backing,
	};

	execenv.run(&code);
}
