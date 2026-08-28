// SPDX-License-Identifier: Apache-2.0

use std::fmt;

/// Stable machine-readable verifier failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorCode {
    FrameTooLarge,
    InvalidJson,
    DuplicateKey,
    UnknownField,
    ResourceLimit,
    UnexpectedMessage,
    LegacyMessage,
    PendingStage,
    InvalidStageToken,
    IncompatibleWelcome,
    BindingMismatch,
    InvalidRegistry,
    InvalidAddress,
    InvalidPrivateLinkage,
    InvalidBaseline,
    InvalidDelta,
    FrontierMismatch,
    InvalidEntity,
    NonCanonicalOrder,
    InvalidFloat,
    HashMismatch,
    Serialization,
}

impl ErrorCode {
    /// Stable lowercase identifier suitable for logs and protocol adapters.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FrameTooLarge => "frame_too_large",
            Self::InvalidJson => "invalid_json",
            Self::DuplicateKey => "duplicate_key",
            Self::UnknownField => "unknown_field",
            Self::ResourceLimit => "resource_limit",
            Self::UnexpectedMessage => "unexpected_message",
            Self::LegacyMessage => "legacy_message",
            Self::PendingStage => "pending_stage",
            Self::InvalidStageToken => "invalid_stage_token",
            Self::IncompatibleWelcome => "incompatible_welcome",
            Self::BindingMismatch => "binding_mismatch",
            Self::InvalidRegistry => "invalid_registry",
            Self::InvalidAddress => "invalid_address",
            Self::InvalidPrivateLinkage => "invalid_private_linkage",
            Self::InvalidBaseline => "invalid_baseline",
            Self::InvalidDelta => "invalid_delta",
            Self::FrontierMismatch => "frontier_mismatch",
            Self::InvalidEntity => "invalid_entity",
            Self::NonCanonicalOrder => "non_canonical_order",
            Self::InvalidFloat => "invalid_float",
            Self::HashMismatch => "hash_mismatch",
            Self::Serialization => "serialization",
        }
    }
}

/// A verifier error with a stable category and bounded explanatory context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    code: ErrorCode,
    detail: String,
}

impl VerifyError {
    pub(crate) fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        const MAX_DETAIL_BYTES: usize = 512;
        let mut detail = detail.into();
        if detail.len() > MAX_DETAIL_BYTES {
            let mut boundary = MAX_DETAIL_BYTES;
            while !detail.is_char_boundary(boundary) {
                boundary -= 1;
            }
            detail.truncate(boundary);
        }
        Self { code, detail }
    }

    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.detail)
    }
}

impl std::error::Error for VerifyError {}

pub(crate) type Result<T> = std::result::Result<T, VerifyError>;
