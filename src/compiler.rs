use std::io;
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
    fn as_multi_with(&self, v: u32) -> Option<Self>
    where
        T: BfOptimizable,
    {
        use BfInstruc::*;

        // this will actually overflow on a maxxed out u32, to combat this we limit the max size of a stream to around 2 billion instructions
        // security as layers, or something

        let rem_v = T::MAX.into().checked_add(1);

        let loop_use = rem_v.map_or(v, |rem| v % rem);

        Some(match self {
            Inc => IncBy(
                loop_use
                    .try_into()
                    .map_err(|_| panic!("could not convert u32 to T"))
                    .unwrap(),
            ),
            Dec => DecBy(
                loop_use
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

macro_rules! render_c {

  ($Ty:ty, $CINT:expr) => {

    impl BfInstructionStream<$Ty> {
      /// renders this instruction stream to a writer in c
      ///
      /// # Errors
      /// This function returns any errors raised by the `out` parameter
      pub fn render_c(&self, mut out: impl io::Write) -> io::Result<()> {
        let opening_brace = '{';
        let closing_brace = '}';
        let array_init = "{0,}";

        write!(out, "#include <stdio.h>\n#define ARRSIZE {}\nint main() {opening_brace}\n{} arr[ARRSIZE] = {array_init};\n{}* a = arr;\n", self.1, $CINT, $CINT)?;

        for i in self.0.iter() {
          use BfInstruc::*;

          match i {
            Zero => write!(out, "*a = 0;"),
            Inc => write!(out, "++*a;"),
            Dec => write!(out, "--*a;"),
            IncPtr => write!(out, "++a;"),
            DecPtr => write!(out, "--a;"),
            Write => write!(out, "fputc(*a, stdout);"),
            Read => write!(out, "*a = fgetc(stdin); if (feof(stdin)) *a = 0;"),
            LStart(_) => write!(out, "while (*a != 0) {opening_brace}"),
            LEnd(_) => write!(out, "{closing_brace}"),
            IncBy(amount) => write!(out, "*a += {amount};"),
            DecBy(amount) => write!(out, "*a -= {amount};"),
            IncPtrBy(amount) => write!(out, "a += {amount};"),
            DecPtrBy(amount) => write!(out, "a -= {amount};"),
          }?;

          write!(out, "\n")?;
        }

        write!(out, "{closing_brace}")
      }
    }

  };

}

render_c!(u8, "unsigned char");
render_c!(u16, "unsigned short");
render_c!(u32, "unsigned int");

pub struct BfInstructionStream<T>(Vec<BfInstruc<T>>, usize);

impl<T: BfOptimizable> BfInstructionStream<T> {
    /// Returns a brainfuck stream fully optimized and run ready from brainfuck text
    ///
    /// # Errors
    /// This function will error if while compiling the loop instructions are malformed by having a mismatched count or by having a loop end instruction without a start instruction
    pub fn optimized_from_text(v: impl Iterator<Item = u8>) -> Result<Self, BfCompError> {
        let mut new = Self(Self::bf_to_stream(v), 0);

        let array_len: u32 = new
            .iter()
            .fold(0, |accu, x| {
                if let BfInstruc::IncPtr = x {
                    accu + 1
                } else {
                    accu
                }
            })
            .max(30_000);

        new.1 = array_len
            .try_into()
            .expect("16 bit platforms not supported");

        if new.len() > (isize::MAX as usize) {
            return Err(BfCompError::Overflow);
        }

        // run optimization passes
        new.group_common_bf();
        new.static_optimize();
        new.insert_bf_jump_points()?;

        Ok(new)
    }

    /// returns a statically guessed array size that would work best for this brainfuck stream
    #[must_use]
    pub fn reccomended_array_size(&self) -> usize {
        self.1
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
                } else {
                    stream[newlen] = stream[i].as_multi_with(ctr).unwrap();
                }
            } else {
                stream[newlen] = stream[i].clone();
            }

            newlen += 1;
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
        use BfInstruc::*;

        const OPT_COUNT: usize = 2;

        let v = &mut self.0;

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
            // will not panic as we are iterating the stream length and never truncating
            #[allow(clippy::match_on_vec_items)]
            match stream[idx] {
                BfInstruc::LStart(_) => {
                    stack.push(idx);
                }
                BfInstruc::LEnd(_) => {
                    if let Some(v) = stack.pop() {
                        stream[v] = BfInstruc::LStart(
                            u32::try_from(idx).expect("u32 overflowed size of usize"),
                        );
                        stream[idx] = BfInstruc::LEnd(
                            u32::try_from(v).expect("u32 overflowed size of usize"),
                        );
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
        let stream_len = stream.len();
        Self(stream, stream_len)
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
        &self.0
    }
}
