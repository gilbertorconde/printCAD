//! What the app keeps of a document's file apart from the window: the
//! recovery copies a crash leaves and the op logs ending unsaved
//! (`recovery`), what wrote a file and whether it can be written in place
//! (`FileSafety`, the save-over question), the unsaved-edits question's
//! answers (`Unsaved`, `Then`) and a document's name from its file's.

mod names;
pub mod recovery;
mod safety;
mod unsaved;

pub use names::*;
pub use safety::*;
pub use unsaved::*;
