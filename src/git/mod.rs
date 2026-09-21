//! Git layer

pub mod branch;
mod compact;
pub mod commit;
pub mod diff;
pub mod extensions;
pub mod graph;
pub mod operations;
pub mod repository;
pub mod tag;

pub use branch::BranchInfo;
pub use commit::CommitInfo;
pub use diff::{
    CommitDiffInfo, DiffHunkContent, DiffLineContent, DiffLineOrigin, FileChangeKind,
    FileDiffContent, FileDiffInfo,
};
pub use extensions::configure_git_extensions;
pub use graph::{build_graph, build_graph_with_options};
pub use repository::{GitRepository, StageState, WorkingTreeStatus};
pub use tag::TagInfo;
