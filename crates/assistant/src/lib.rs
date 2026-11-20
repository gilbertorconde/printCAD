//! The assistant's model, apart from the window: chats with AI agents and
//! how what they stream folds into them (`Chat`), the chats kept with each
//! document's file (`chat_store`), `@` mentions (`mentions`) and the tool
//! calls agents make (`ToolRequest`, `Approval`).

mod approval;
mod chat;
pub mod chat_store;
pub mod mentions;

pub use approval::*;
pub use chat::*;
