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

//! Type definitions for command execution.

use crate::Env;
use nix::sys::signal::Signal;
use nix::sys::wait::WaitStatus;
use std::ffi::c_int;
use std::ops::ControlFlow::{self, Break};
use std::process::ExitCode;
use std::process::Termination;
use yash_syntax::source::Location;

/// Resultant string of word expansion.
///
/// A field is a string accompanied with the original word location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    /// String value of the field.
    pub value: String,
    /// Location of the word this field resulted from.
    pub origin: Location,
}

impl Field {
    /// Creates a new field with a dummy origin location.
    ///
    /// The value of the resulting field will be `value.into()`.
    /// The origin of the field will be created by [`Location::dummy`] with a
    /// clone of the value.
    #[inline]
    pub fn dummy<S: Into<String>>(value: S) -> Field {
        fn with_value(value: String) -> Field {
            let origin = Location::dummy(value.clone());
            Field { value, origin }
        }
        with_value(value.into())
    }

    /// Creates an array of fields with dummy origin locations.
    ///
    /// This function calls [`dummy`](Self::dummy) to create the results.
    pub fn dummies<I, S>(values: I) -> Vec<Field>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        values.into_iter().map(Self::dummy).collect()
    }
}

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

/// Number that summarizes the result of command execution.
///
/// An exit status is an integer returned from a utility (or command) when
/// executed. It usually is a summarized result of the execution.  Many
/// utilities return an exit status of zero when successful and non-zero
/// otherwise.
///
/// In the shell language, the special parameter `$?` expands to the exit status
/// of the last executed command. Exit statuses also affect the behavior of some
/// compound commands.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExitStatus(pub c_int);

impl std::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<c_int> for ExitStatus {
    fn from(value: c_int) -> ExitStatus {
        ExitStatus(value)
    }
}

impl From<ExitStatus> for c_int {
    fn from(exit_status: ExitStatus) -> c_int {
        exit_status.0
    }
}

/// Converts a signal to the corresponding exit status.
///
/// POSIX requires the exit status to be greater than 128. The current
/// implementation returns `signal_number + 384`.
impl From<Signal> for ExitStatus {
    fn from(signal: Signal) -> Self {
        Self::from(signal as c_int + 0x180)
    }
}

/// Converts the exit status to `ExitCode`.
///
/// Note that `ExitCode` only supports exit statuses in the range of 0 to 255.
/// Only the lowest 8 bits of the exit status are used in the conversion.
impl Termination for ExitStatus {
    fn report(self) -> ExitCode {
        (self.0 as u8).into()
    }
}

/// Error returned when a [`WaitStatus`] could not be converted to an
/// [`ExitStatus`]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StillAliveError;

/// Converts a `WaitStatus` to an `ExitStatus` if the status is `Exited`,
/// `Signaled`, or `Stopped`.
impl TryFrom<WaitStatus> for ExitStatus {
    type Error = StillAliveError;
    fn try_from(status: WaitStatus) -> std::result::Result<Self, StillAliveError> {
        match status {
            WaitStatus::Exited(_, exit_status) => Ok(ExitStatus(exit_status)),
            WaitStatus::Signaled(_, signal, _) | WaitStatus::Stopped(_, signal) => {
                Ok(ExitStatus::from(signal))
            }
            _ => Err(StillAliveError),
        }
    }
}

/// Converts an exit status to the corresponding signal.
///
/// If there is a signal such that
/// `exit_status == ExitStatus::from(signal)`,
/// the signal is returned.
/// The same if the exit status is the lowest 8 bits of such an exit status.
/// The signal is also returned if the exit status is a signal number itself.
/// Otherwise, an error is returned.
impl TryFrom<ExitStatus> for Signal {
    type Error = nix::Error;
    fn try_from(exit_status: ExitStatus) -> nix::Result<Signal> {
        Signal::try_from(exit_status.0 - 0x180)
            .or_else(|_| Signal::try_from(exit_status.0 - 0x80))
            .or_else(|_| Signal::try_from(exit_status.0))
    }
}

impl ExitStatus {
    /// Exit status of 0: success.
    pub const SUCCESS: ExitStatus = ExitStatus(0);

    /// Exit status of 1: failure.
    pub const FAILURE: ExitStatus = ExitStatus(1);

    /// Exit status of 2: error severer than failure.
    pub const ERROR: ExitStatus = ExitStatus(2);

    /// Exit Status of 126: command not executable.
    pub const NOEXEC: ExitStatus = ExitStatus(126);

    /// Exit status of 127: command not found.
    pub const NOT_FOUND: ExitStatus = ExitStatus(127);

    /// Returns true if and only if `self` is zero.
    pub const fn is_successful(&self) -> bool {
        self.0 == 0
    }
}

/// Result of interrupted command execution.
///
/// `Divert` implements `Ord`. Values are ordered by severity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Divert {
    /// Continue the current loop.
    Continue {
        /// Number of loops to break before continuing.
        ///
        /// `0` for continuing the innermost loop, `1` for one-level outer, and so on.
        count: usize,
    },

    /// Break the current loop.
    Break {
        /// Number of loops to break.
        ///
        /// `0` for breaking the innermost loop, `1` for one-level outer, and so on.
        count: usize,
    },

    /// Return from the current function or script.
    Return(Option<ExitStatus>),

    /// Interrupt the current shell execution environment.
    ///
    /// This is the same as `Exit` in a non-interactive shell: it makes the
    /// shell exit after executing the EXIT trap, if any. If this is used inside
    /// the EXIT trap, the shell will exit immediately.
    ///
    /// In an interactive shell, this will abort the currently executed command
    /// and resume prompting for a next command line.
    Interrupt(Option<ExitStatus>),

    /// Exit from the current shell execution environment.
    ///
    /// This makes the shell exit after executing the EXIT trap, if any.
    /// If this is used inside the EXIT trap, the shell will exit immediately.
    Exit(Option<ExitStatus>),

    /// Exit from the current shell execution environment immediately.
    ///
    /// This makes the shell exit without executing the EXIT trap.
    Abort(Option<ExitStatus>),
}

impl Divert {
    /// Returns the exit status associated with the `Divert`.
    ///
    /// Returns the variant's value if `self` is `Exit` or `Interrupt`;
    /// otherwise, `None`.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        use Divert::*;
        match self {
            Continue { .. } | Break { .. } => None,
            Return(exit_status)
            | Interrupt(exit_status)
            | Exit(exit_status)
            | Abort(exit_status) => *exit_status,
        }
    }
}

/// Result of command execution.
///
/// If the command was interrupted in the middle of execution, the result value
/// will be a `Break` having a [`Divert`] value which specifies what to execute
/// next.
pub type Result<T = ()> = ControlFlow<Divert, T>;

/// Applies the `ErrExit` shell option to the result.
///
/// If the `ErrExit` option is on in `env.options`, `env.stack` contains no
/// `Frame::Condition`, and the `result` is `Divert::Interrupt`, then the result
/// is converted to `Divert::Exit`.
pub fn apply_errexit<T>(result: Result<T>, env: &Env) -> Result<T> {
    match result {
        Break(Divert::Interrupt(exit_status)) if env.errexit_is_applicable() => {
            Break(Divert::Exit(exit_status))
        }

        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::option::Option::ErrExit;
    use crate::option::State::On;
    use crate::stack::Frame;

    #[test]
    fn apply_errexit_applicable() {
        let mut env = Env::new_virtual();
        env.options.set(ErrExit, On);
        let subject: Result = Break(Divert::Interrupt(Some(ExitStatus(42))));
        let result = apply_errexit(subject, &env);
        assert_eq!(result, Break(Divert::Exit(Some(ExitStatus(42)))));
    }

    #[test]
    fn apply_errexit_to_non_interrupt() {
        let mut env = Env::new_virtual();
        env.options.set(ErrExit, On);
        let subject: Result = Break(Divert::Return(None));
        let result = apply_errexit(subject, &env);
        assert_eq!(result, subject);
    }

    #[test]
    fn apply_errexit_in_condition_context() {
        let mut env = Env::new_virtual();
        env.options.set(ErrExit, On);
        let env = env.push_frame(Frame::Condition);
        let subject: Result = Break(Divert::Interrupt(Some(ExitStatus(42))));
        let result = apply_errexit(subject, &env);
        assert_eq!(result, subject);
    }

    #[test]
    fn apply_errexit_with_disabled_option() {
        let env = Env::new_virtual();
        let subject: Result = Break(Divert::Interrupt(Some(ExitStatus(42))));
        let result = apply_errexit(subject, &env);
        assert_eq!(result, subject);
    }

    #[test]
    fn signal_try_from_exit_status() {
        let result = Signal::try_from(ExitStatus(0));
        assert!(result.is_err(), "result = {result:?}");

        assert_eq!(
            Signal::try_from(ExitStatus(Signal::SIGINT as c_int)),
            Ok(Signal::SIGINT)
        );

        let mut exit_status = ExitStatus::from(Signal::SIGTERM);
        exit_status.0 &= 0xFF;
        assert_eq!(Signal::try_from(exit_status), Ok(Signal::SIGTERM));

        assert_eq!(
            Signal::try_from(ExitStatus::from(Signal::SIGHUP)),
            Ok(Signal::SIGHUP)
        );
    }
}
