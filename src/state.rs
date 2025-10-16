//! Base types with common implementations

/// The base state of a valid bf program
pub struct BfState<C, I, O> {
    pub ptr: usize,
    pub cells: Box<[C]>,
    pub read: I,
    pub write: O,
}
