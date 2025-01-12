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

//! Methods about passing [source](yash_syntax::source) code to the
//! [parser](yash_syntax::parser).
//!
//! This module extends [`yash_syntax::input`] with input functions that are
//! implemented depending on the environment.

use crate::io::Fd;
use crate::option::State;
use crate::system::SharedSystem;
use async_trait::async_trait;
use std::cell::Cell;
use std::rc::Rc;
use std::slice::from_mut;

#[doc(no_inline)]
pub use yash_syntax::input::*;

// TODO Redefine Stdin as FdReader to support FDs other than stdin

/// Input function that reads from the standard input.
///
/// An instance of `Stdin` contains a [`SharedSystem`] to read the input from,
/// as well as the current line number.
///
/// Although `Stdin` implements `Clone`, it does not mean you can create and
/// keep a copy of a `Stdin` instance to replay the input later. Since both the
/// original and clone share the same `SharedSystem`, reading a line from one
/// instance will affect the next read from the other instance.
#[derive(Clone, Debug)]
pub struct Stdin {
    /// System to interact with the FD
    system: SharedSystem,
    /// Whether lines read are echoed to stderr
    echo: Option<Rc<Cell<State>>>,
}

impl Stdin {
    /// Creates a new `Stdin` instance.
    pub fn new(system: SharedSystem) -> Self {
        Stdin { system, echo: None }
    }

    /// Sets the "echo" flag.
    ///
    /// You can use this setter function to set a shared option state that
    /// controls whether the input function echoes lines it reads to the
    /// standard error. If `echo` is `None` or some shared cell containing
    /// `Off`, the function does not echo. If a cell has `On`, the function
    /// prints every line it reads to the standard error.
    ///
    /// This option implements the behavior of the `verbose` shell option. You
    /// can change the state of the shared cell through the lifetime of the
    /// input function to reflect the option dynamically changed, which will
    /// affect the next `next_line` call.
    pub fn set_echo(&mut self, echo: Option<Rc<Cell<State>>>) {
        self.echo = echo;
    }
}

#[async_trait(?Send)]
impl Input for Stdin {
    async fn next_line(&mut self, _context: &Context) -> Result {
        // TODO Read many bytes at once if seekable

        let mut bytes = Vec::new();
        loop {
            let mut byte = 0;
            match self.system.read_async(Fd::STDIN, from_mut(&mut byte)).await {
                // End of input
                Ok(0) => break,

                Ok(count) => {
                    assert_eq!(count, 1);
                    bytes.push(byte);
                    if byte == b'\n' {
                        break;
                    }
                }

                Err(errno) => return Err(std::io::Error::from_raw_os_error(errno as i32)),
            }
        }

        // TODO Maybe we should report invalid UTF-8 bytes rather than ignoring them
        let line = String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into());

        if let Some(echo) = &self.echo {
            if echo.get() == State::On {
                let _ = self.system.write_all(Fd::STDERR, line.as_bytes()).await;
            }
        }

        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::r#virtual::FileBody;
    use crate::system::r#virtual::VirtualSystem;
    use crate::system::Errno;
    use assert_matches::assert_matches;
    use futures_util::FutureExt;

    #[test]
    fn stdin_empty() {
        let system = VirtualSystem::new();
        let system = SharedSystem::new(Box::new(system));
        let mut stdin = Stdin::new(system);

        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let line = result.unwrap();
        assert_eq!(line, "");
    }

    #[test]
    fn stdin_one_line() {
        let system = VirtualSystem::new();
        {
            let state = system.state.borrow_mut();
            let file = state.file_system.get("/dev/stdin").unwrap();
            file.borrow_mut().body = FileBody::new(*b"echo ok\n");
        }
        let system = SharedSystem::new(Box::new(system));
        let mut stdin = Stdin::new(system);

        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let line = result.unwrap();
        assert_eq!(line, "echo ok\n");
        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let line = result.unwrap();
        assert_eq!(line, "");
    }

    #[test]
    fn stdin_many_lines() {
        let system = VirtualSystem::new();
        {
            let state = system.state.borrow_mut();
            let file = state.file_system.get("/dev/stdin").unwrap();
            file.borrow_mut().body = FileBody::new(*b"#!/bin/sh\necho ok\nexit");
        }
        let system = SharedSystem::new(Box::new(system));
        let mut stdin = Stdin::new(system);

        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let line = result.unwrap();
        assert_eq!(line, "#!/bin/sh\n");
        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let line = result.unwrap();
        assert_eq!(line, "echo ok\n");
        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let line = result.unwrap();
        assert_eq!(line, "exit");
        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let line = result.unwrap();
        assert_eq!(line, "");
    }

    #[test]
    fn stdin_error() {
        let mut system = VirtualSystem::new();
        system.current_process_mut().close_fd(Fd::STDIN);
        let system = SharedSystem::new(Box::new(system));
        let mut stdin = Stdin::new(system);

        let result = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let error = result.unwrap_err();
        assert_eq!(error.raw_os_error(), Some(Errno::EBADF as i32));
    }

    #[test]
    fn echo_off() {
        let system = VirtualSystem::new();
        let state = Rc::clone(&system.state);
        {
            let state = state.borrow();
            let file = state.file_system.get("/dev/stdin").unwrap();
            file.borrow_mut().body = FileBody::new(*b"one\ntwo");
        }
        let system = SharedSystem::new(Box::new(system));
        let mut stdin = Stdin::new(system);
        stdin.set_echo(Some(Rc::new(Cell::new(State::Off))));

        let _ = stdin.next_line(&Context::default()).now_or_never().unwrap();
        let state = state.borrow();
        let file = state.file_system.get("/dev/stderr").unwrap();
        assert_matches!(&file.borrow().body, FileBody::Regular { content, .. } => {
            assert_eq!(content, &[]);
        });
    }

    #[test]
    fn echo_on() {
        let system = VirtualSystem::new();
        let state = Rc::clone(&system.state);
        {
            let state = state.borrow();
            let file = state.file_system.get("/dev/stdin").unwrap();
            file.borrow_mut().body = FileBody::new(*b"one\ntwo");
        }
        let system = SharedSystem::new(Box::new(system));
        let mut stdin = Stdin::new(system);
        stdin.set_echo(Some(Rc::new(Cell::new(State::On))));

        let _ = stdin.next_line(&Context::default()).now_or_never().unwrap();
        {
            let state = state.borrow();
            let file = state.file_system.get("/dev/stderr").unwrap();
            assert_matches!(&file.borrow().body, FileBody::Regular { content, .. } => {
                assert_eq!(content, b"one\n");
            });
        }
        let _ = stdin.next_line(&Context::default()).now_or_never().unwrap();
        {
            let state = state.borrow();
            let file = state.file_system.get("/dev/stderr").unwrap();
            assert_matches!(&file.borrow().body, FileBody::Regular { content, .. } => {
                assert_eq!(content, b"one\ntwo");
            });
        }
    }
}
