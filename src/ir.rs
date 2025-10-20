//! An intermediate DAG representation for a BF programs optimization stage

use std::ops::Range;


pub enum Token {
    Zero,
    Inc(u32),
    Dec(u32),
    IncPtr(usize),
    DecPtr(usize),
    Read,
    Write,
    LStart,
    LEnd,
}


pub struct MulArg {
    offset: isize,
    change: i64,
}

pub enum ITree {
    Zero,
    Mul(Range<isize>, Vec<MulArg>),
    Inc(u32),
    Dec(u32),
    IncPtr(usize),
    DecPtr(usize),
    Read,
    Write,
    Loop(Vec<ITree>),
    WriteLoop(Vec<ITree>),
}

impl ITree {
    fn terminates(&self) -> bool {
        !matches!(self, Self::Loop(_) | Self::WriteLoop(_))
    }

    fn zero_in_loop(this: &[Self]) -> bool {
        matches!(this, [Self::Inc(1)] | [Self::Dec(1)])
    }

    fn is_writeloop(this: &[Self]) -> bool {
        this.len() < 32
            && this
                .iter()
                .all(|c| c.terminates() & !matches!(c, Self::Read))
            && this.iter().any(|c| matches!(c, Self::Write))
    }

    fn as_multiply(this: &[Self]) -> Option<(Range<isize>, Vec<MulArg>)> {
        const Z_OFFSET: usize = 32;

        let mut minivm = [0i64; 64];
        let mut idx = Z_OFFSET;

        let mut bounds = 0..0isize;

        for node in this {
            match node {
                Self::Zero
                | Self::Mul(_, _)
                | Self::Read
                | Self::Write
                | Self::Loop(_)
                | Self::WriteLoop(_) => return None,
                Self::Inc(by) => {
                    let Some(inc) = minivm[idx].checked_add(i64::from(*by)) else {
                        return None;
                    };

                    minivm[idx] = inc;
                }
                Self::Dec(by) => {
                    let Some(dec) = minivm[idx].checked_sub(i64::from(*by)) else {
                        return None;
                    };

                    minivm[idx] = dec;
                }
                Self::IncPtr(by) => {
                    let Some(incptr) = idx.checked_add(*by) else {
                        return None;
                    };

                    if incptr < minivm.len() {
                        idx = incptr;

                        bounds.end = core::cmp::max(bounds.end, (idx as isize) - Z_OFFSET as isize);
                    } else {
                        return None;
                    }
                }
                Self::DecPtr(by) => {
                    let Some(decptr) = idx.checked_sub(*by) else {
                        return None;
                    };

                    idx = decptr;

                    bounds.start = core::cmp::min(bounds.start, (idx as isize) - Z_OFFSET as isize);
                }
            }
        }

        if (idx == Z_OFFSET) & (minivm[Z_OFFSET] == -1) {
            let mut out = vec![];

            for (idx, cell) in minivm.into_iter().enumerate() {
                if (cell != 0) & (idx != Z_OFFSET) {
                    out.push(MulArg {
                        offset: idx as isize - Z_OFFSET as isize,
                        change: cell,
                    });
                }
            }

            Some((bounds, out))
        } else {
            None
        }
    }
}

pub fn rewrite_zero(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(children) = node {
            if ITree::zero_in_loop(&children) {
                *node = ITree::Zero;
            } else {
                rewrite_zero(children);
            }
        }
    }
}

pub fn rewrite_multiply(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(children) = node {
            if let Some(mulargs) = ITree::as_multiply(children) {
                *node = ITree::Mul(mulargs.0, mulargs.1);
            } else {
                rewrite_multiply(children);
            }
        }
    }
}

pub fn rewrite_write_loops(tree: &mut [ITree]) {
    for node in tree {
        if let ITree::Loop(children) = node {
            if ITree::is_writeloop(&children) {
                *node = ITree::WriteLoop(core::mem::take(children));
            } else {
                rewrite_write_loops(children);
            }
        }
    }
}
