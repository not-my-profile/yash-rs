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

//! Type definitions for I/O.

use std::os::unix::io::RawFd;

/// File descriptor.
///
/// This is the `newtype` pattern applied to [`RawFd`], which is merely a type
/// alias.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Fd(pub RawFd);

impl Fd {
    /// File descriptor for the standard input.
    pub const STDIN: Fd = Fd(0);
    /// File descriptor for the standard output.
    pub const STDOUT: Fd = Fd(1);
    /// File descriptor for the standard error.
    pub const STDERR: Fd = Fd(2);
}

impl From<RawFd> for Fd {
    fn from(raw_fd: RawFd) -> Fd {
        Fd(raw_fd)
    }
}