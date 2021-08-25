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

//! Implementations of [`Handle`].

use crate::ExitStatus;
use crate::Handle;
use async_trait::async_trait;
use yash_env::Env;

#[async_trait(?Send)]
impl Handle<crate::expansion::Error> for Env {
    /// Prints an error message and sets the exit status to non-zero.
    ///
    /// This function handles an expansion error by printing an error message
    /// that describes the error to the standard error and setting the exit
    /// status to [`ExitStatus::ERROR`]. Note that other POSIX-compliant
    /// implementations may use different non-zero exit statuses.
    async fn handle(&mut self, error: crate::expansion::Error) -> super::Result {
        use crate::expansion::ErrorCause::*;
        // TODO Localize the message
        // TODO Pretty-print the error location
        match error.cause {
            Dummy(message) => {
                self.print_error(&format_args!("dummy error: {}", message))
                    .await
            }
        };
        self.exit_status = ExitStatus::ERROR;
        Ok(())
    }
}