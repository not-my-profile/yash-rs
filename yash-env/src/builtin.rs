// This file is part of yash, an extended POSIX shell.
// Copyright (C) 2021 WATANABE Yuki
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Type definitions for built-in utilities.
//!
//! This module provides data types for defining built-in utilities.
//!
//! Note that concrete implementations of built-ins are not included in the
//! `yash_env` crate. For implementations of specific built-ins like `cd` and
//! `export`, see the `yash_builtin` crate.

#[cfg(doc)]
use crate::semantics::Divert;
use crate::semantics::ExitStatus;
use crate::semantics::Field;
use crate::Env;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

/// Types of built-in utilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Type {
    /// Special built-in.
    ///
    /// Special built-in utilities are treated differently from regular built-ins.
    /// Especially, special built-ins are found in the first stage of command
    /// search and cannot be overridden by functions or external utilities. Many
    /// errors in special built-ins force the shell to exit.
    Special,

    /// Intrinsic regular built-in.
    ///
    /// Like special built-ins, intrinsic built-ins are not subject to $PATH in
    /// command search; They are always found regardless of whether there is a
    /// corresponding external utility in $PATH. However, intrinsic built-ins can
    /// still be overridden by functions.
    Intrinsic,

    /// Non-intrinsic regular built-in.
    ///
    /// Non-intrinsic built-ins are much like external utilities; They must be
    /// found in $PATH in order to be executed.
    NonIntrinsic,
    // /// Non-portable extension.
    // ///
    // /// Extension built-ins are intrinsic built-ins that are not defined in POSIX.
    // /// They are treated like non-existing utilities in the POSIX mode.
    // TODO Extension,
}

/// Result of built-in utility execution.
///
/// The result type contains an exit status and optional flags that may affect
/// the behavior of the shell following the built-in execution.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[must_use]
pub struct Result {
    exit_status: ExitStatus,
    divert: crate::semantics::Result,
    should_retain_redirs: bool,
}

impl Result {
    /// Creates a new result.
    pub const fn new(exit_status: ExitStatus) -> Self {
        Self {
            exit_status,
            divert: crate::semantics::Result::Continue(()),
            should_retain_redirs: false,
        }
    }

    /// Creates a new result with a [`Divert`].
    #[inline]
    pub const fn with_exit_status_and_divert(
        exit_status: ExitStatus,
        divert: crate::semantics::Result,
    ) -> Self {
        Self {
            exit_status,
            divert,
            should_retain_redirs: false,
        }
    }

    /// Returns the exit status of this result.
    ///
    /// The return value is the argument to the previous invocation of
    /// [`new`](Self::new) or [`set_exit_status`](Self::set_exit_status).
    #[inline]
    #[must_use]
    pub const fn exit_status(&self) -> ExitStatus {
        self.exit_status
    }

    /// Sets the exit status of this result.
    ///
    /// See [`exit_status`](Self::exit_status()).
    #[inline]
    pub fn set_exit_status(&mut self, exit_status: ExitStatus) {
        self.exit_status = exit_status
    }

    /// Returns an optional [`Divert`] to be taken.
    ///
    /// The return value is the argument to the previous invocation of
    /// [`set_divert`](Self::set_divert). The default is `Continue(())`.
    #[inline]
    #[must_use]
    pub const fn divert(&self) -> crate::semantics::Result {
        self.divert
    }

    /// Sets a [`Divert`].
    ///
    /// See [`divert`](Self::divert()).
    #[inline]
    pub fn set_divert(&mut self, divert: crate::semantics::Result) {
        self.divert = divert;
    }

    /// Tests whether the caller should retain redirections.
    ///
    /// Usually, the shell reverts redirections applied to a built-in after
    /// executing it. However, redirections applied to a successful `exec`
    /// built-in should persist. To make it happen, the `exec` built-in calls
    /// [`retain_redirs`](Self::retain_redirs), and this function returns true.
    /// In that case, the caller of the built-in should take appropriate actions
    /// to preserve the effect of the redirections.
    #[inline]
    pub const fn should_retain_redirs(&self) -> bool {
        self.should_retain_redirs
    }

    /// Flags that redirections applied to the built-in should persist.
    ///
    /// Calling this function makes
    /// [`should_retain_redirs`](Self::should_retain_redirs) return true.
    /// [`clear_redirs`](Self::clear_redirs) cancels the effect of this
    /// function.
    #[inline]
    pub fn retain_redirs(&mut self) {
        self.should_retain_redirs = true;
    }

    /// Cancels the effect of [`retain_redirs`](Self::retain_redirs).
    #[inline]
    pub fn clear_redirs(&mut self) {
        self.should_retain_redirs = false;
    }
}

impl Default for Result {
    fn default() -> Self {
        Self::new(ExitStatus::default())
    }
}

impl From<ExitStatus> for Result {
    fn from(exit_status: ExitStatus) -> Self {
        Self::new(exit_status)
    }
}

/// Type of functions that implement the behavior of a built-in.
///
/// The function takes two arguments.
/// The first is an environment in which the built-in is executed.
/// The second is arguments to the built-in
/// (not including the leading command name word).
pub type Main = fn(&mut Env, Vec<Field>) -> Pin<Box<dyn Future<Output = Result> + '_>>;

/// Built-in utility definition.
#[derive(Clone, Copy)]
pub struct Builtin {
    /// Type of the built-in.
    pub r#type: Type,
    /// Function that implements the behavior of the built-in.
    pub execute: Main,
}

impl Debug for Builtin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builtin")
            .field("type", &self.r#type)
            .finish_non_exhaustive()
    }
}
