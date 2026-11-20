//! What the app keeps of workbench packages and its own releases, apart
//! from the window: the packages loaded at start and the threads that
//! install, update and check them (`PackageNews`), the workbench stores'
//! lists (`StoreView`), and whether a newer printCAD is out
//! (`ReleaseCheck`).

mod packages;
mod release;

pub use packages::*;
pub use release::*;
