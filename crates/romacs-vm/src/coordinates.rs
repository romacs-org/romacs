//! Coordinate spaces never convert implicitly. Lisp presents scalar positions;
//! text storage and edits use absolute UTF-8 byte offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteOffset(pub usize);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CharOffset(pub usize);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphemeOffset(pub usize);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DisplayColumn(pub usize);
