// SPDX-License-Identifier: MIT

//! Typed loader and threshold failures. Display text never includes paths,
//! file bytes, or manifest bodies.

/// Failure while reading a staging tree or building a threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Filesystem failure. The message never includes the path.
    #[error("I/O error")]
    Io,

    /// The staging root, or a component that must be a directory, is not one.
    #[error("staging path is not a directory")]
    NotADirectory,

    /// A directory component was a symlink. The walk does not follow it.
    #[error("refusing to follow a symlink")]
    Symlink,

    /// `read_dir` yielded more names than [`crate::LoadLimits::max_entries_scanned`].
    #[error("directory entry count exceeds limit of {max}")]
    TooManyEntries {
        /// The cap that was crossed.
        max: u64,
    },

    /// A [`crate::BuildId`] could not be turned into a safe directory name.
    #[error("invalid build identifier")]
    InvalidBuildId,

    /// [`crate::AgreementThreshold::new`] rejected the numbers.
    #[error("invalid agreement threshold")]
    InvalidThreshold,

    /// The publish directory is the staging input or a path inside it.
    #[error("output path is inside the staging input")]
    InsideStaging,

    /// An output file is already present. This writer does not overwrite.
    #[error("output file already exists")]
    AlreadyExists,

    /// The output path is the same file as an input.
    #[error("output path is the same file as an input")]
    SameAsInput,

    /// The output path is a directory, fifo, or other non-regular file.
    #[error("output path is not a regular file")]
    NotRegular,

    /// `write_record_set` had no record whose pipeline hash reached consensus.
    #[error("no records reached consensus")]
    NoConsensusRecords,

    /// The candidate record file failed the strict record loader.
    #[error("record file rejected")]
    RecordRejected,

    /// The record file's build labels are not the consensus build.
    #[error("record build does not match the consensus build")]
    RecordBuildMismatch,

    /// The coverage index could not be encoded.
    #[error("coverage index rejected")]
    CoverageRejected,
}
