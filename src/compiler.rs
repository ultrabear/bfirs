use std::num::NonZeroU32;
use thiserror::Error;

#[repr(u8)]
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum BfInstruc<CellSize> {
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
	IncPtrBy(NonZeroU32),
	DecPtrBy(NonZeroU32),
}

impl<T> TryFrom<u8> for BfInstruc<T> {
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

impl<T> BfInstruc<T> {
	fn as_multi_with(&self, v: u32) -> Option<BfInstruc<T>>
	where
		T: BfOptimizable,
	{
		use BfInstruc::*;

		// this will actually overflow on a maxxed out u32, to combat this we limit the max size of a stream to around 2 billion instructions
		// security as layers, or something
		Some(match self {
			Inc => IncBy(
				(v % (T::MAX.into() + 1))
					.try_into()
					.map_err(|_| panic!("could not convert u32 to T"))
					.unwrap(),
			),
			Dec => DecBy(
				(v % (T::MAX.into() + 1))
					.try_into()
					.map_err(|_| panic!("could not convert u32 to T"))
					.unwrap(),
			),
			IncPtr => IncPtrBy(NonZeroU32::new(v).unwrap()),
			DecPtr => DecPtrBy(NonZeroU32::new(v).unwrap()),
			_ => None?,
		})
	}
	fn is_multi_optimizable(&self) -> bool {
		use BfInstruc::*;
		matches!(self, Inc | Dec | IncPtr | DecPtr)
	}
}

#[derive(Copy, Clone, Debug, Error)]
pub enum BfCompError {
	#[error(
		"the count of loop start instructions does not match the count of loop end instructions"
	)]
	LoopCountMismatch,
	#[error("loop end instruction was encountered before loop start instruction to complete it")]
	LoopEndBeforeLoopStart,
	#[error("overflowed maximum code size allowed by interpreter")]
	Overflow,
}

pub trait BfOptimizable:
	Clone + Eq + Into<u32> + TryFrom<u32> + Ord + std::ops::Rem<Self, Output = Self>
{
	const MAX: Self;
}

macro_rules! make_optimizable {
	($Ty:ty) => {
		impl BfOptimizable for $Ty {
			const MAX: $Ty = <$Ty>::MAX;
		}
	};
}

make_optimizable!(u8);
make_optimizable!(u16);
make_optimizable!(u32);

pub struct BfInstructionStream<T>(Vec<BfInstruc<T>>);

impl<T: BfOptimizable> BfInstructionStream<T> {
	pub fn optimized_from_text(v: impl Iterator<Item = u8>) -> Result<Self, BfCompError> {
		let mut new = Self(Self::bf_to_stream(v));

		if new.len() > (isize::MAX as usize) {
			return Err(BfCompError::Overflow);
		}

		// run optimization passes
		new.group_common_bf();
		new.static_optimize();
		new.insert_bf_jump_points()?;

		Ok(new)
	}

	// without this inline attr it fails to inline this function into the mainloop, preventing a considerable speedup
	#[inline]
	fn group_common_bf(&mut self) {
		let stream = &mut self.0;

		let mut newlen = 0usize;

		let mut i = 0usize;
		while i < stream.len() {
			if stream[i].is_multi_optimizable() {
				let mut ctr = 1;

				while (i + 1 < stream.len()) && (stream[i] == stream[i + 1]) {
					i += 1;
					ctr += 1;
				}

				if ctr == 1 {
					stream[newlen] = stream[i].clone();
					newlen += 1;
				} else {
					stream[newlen] = stream[i].as_multi_with(ctr).unwrap();
					newlen += 1
				}
			} else {
				stream[newlen] = stream[i].clone();
				newlen += 1;
			}

			i += 1;
		}

		stream.truncate(newlen);
	}
}

impl<T> BfInstructionStream<T> {
	fn bf_to_stream(v: impl Iterator<Item = u8>) -> Vec<BfInstruc<T>> {
		v.filter_map(|byte| BfInstruc::try_from(byte).ok())
			.collect()
	}

	fn static_optimize(&mut self)
	where
		T: Eq + Clone,
	{
		let v = &mut self.0;

		const OPT_COUNT: usize = 2;

		use BfInstruc::*;

		let static_tree: [(&[BfInstruc<T>], BfInstruc<T>); OPT_COUNT] = [
			(&[LStart(0), Dec, LEnd(0)], Zero),
			(&[LStart(0), Inc, LEnd(0)], Zero),
		];

		let mut optimized_count = 1;

		while optimized_count != 0 {
			optimized_count = 0;

			let mut paths = [0usize; OPT_COUNT];

			let mut newidx = 0usize;

			let mut i = 0;
			while i < v.len() {
				let mut optimized = None::<(BfInstruc<T>, usize)>;

				'opt: for (idx, p) in paths.iter_mut().enumerate() {
					if v[i] == static_tree[idx].0[*p] {
						*p += 1;
					} else {
						*p = 0;
						if v[i] == static_tree[idx].0[0] {
							*p += 1;
						}
					}

					if *p == static_tree[idx].0.len() {
						optimized = Some((static_tree[idx].1.clone(), *p));
						break 'opt;
					}
				}

				v[newidx] = v[i].clone();
				newidx += 1;

				if let Some((ins, cnt)) = optimized {
					optimized_count += 1;
					paths = [0; OPT_COUNT];

					newidx -= cnt;
					v[newidx] = ins;
					newidx += 1;
				}

				i += 1;
			}
			v.truncate(newidx);
		}
	}

	fn insert_bf_jump_points(&mut self) -> Result<(), BfCompError> {
		let mut stack = Vec::<usize>::new();

		let stream = &mut self.0;

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
						return Err(BfCompError::LoopEndBeforeLoopStart);
					}
				}
				_ => {}
			}
		}

		if !stack.is_empty() {
			return Err(BfCompError::LoopCountMismatch);
		}

		Ok(())
	}
}

impl<T> From<Vec<BfInstruc<T>>> for BfInstructionStream<T> {
	fn from(stream: Vec<BfInstruc<T>>) -> Self {
		Self(stream)
	}
}

impl<T> From<BfInstructionStream<T>> for Vec<BfInstruc<T>> {
	fn from(stream: BfInstructionStream<T>) -> Self {
		stream.0
	}
}

impl<T> std::ops::Deref for BfInstructionStream<T> {
	type Target = [BfInstruc<T>];

	fn deref(&self) -> &Self::Target {
		self.0.deref()
	}
}
