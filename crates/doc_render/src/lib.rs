//! What the app draws and lays out from a document without a window: the
//! CPU pictures (a save's preview, an animation's frames, the pictures an
//! agent or a script asks for), the PDF writer, faces picked on a mesh and
//! the print layout's packing. Nothing here links the GPU, the window or
//! the UI toolkit, so it runs in workers, headless and in tests alike.

pub mod face_pick;
pub mod pdf;
pub mod print_layout;
pub mod proof;
pub mod thumbnail;

#[cfg(test)]
mod test_support;
