//! Executor selection and resource-domain admission, independent of action identity.

mod admission;
mod selection;

pub use admission::{ExecutionPermit, ExecutorResources};
pub use selection::{Executor, ResourceDomain};
