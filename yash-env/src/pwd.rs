// This file is part of yash, an extended POSIX shell.
// Copyright (C) 2022 WATANABE Yuki
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

//! Working directory path handling

use super::Env;
use crate::system::AtFlags;
use crate::system::AT_FDCWD;
use crate::variable::ReadOnlyError;
use crate::variable::Scope::Global;
use crate::variable::Value::Scalar;
use crate::variable::Variable;
use crate::System;
use nix::sys::stat::FileStat;
use std::ffi::CStr;
use std::ffi::CString;
use std::path::Path;
use thiserror::Error;

/// Tests whether a path contains a dot (`.`) or dot-dot (`..`) component.
fn has_dot_or_dot_dot(path: &str) -> bool {
    path.split('/').any(|c| c == "." || c == "..")
}

/// Tests whether two stats refer to the same file.
fn same_files(a: &FileStat, b: &FileStat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}

/// Error in [`Env::prepare_pwd`]
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PreparePwdError {
    /// Error assigning to the `$PWD` variable
    #[error(transparent)]
    AssignError(#[from] ReadOnlyError),

    /// Error obtaining the current working directory path
    #[error("cannot obtain the current working directory path: {0}")]
    GetCwdError(#[from] nix::Error),
}

impl Env {
    /// Returns the value of the `$PWD` variable if it is correct.
    ///
    /// The variable is correct if:
    ///
    /// - it is a scalar variable,
    /// - its value is a pathname of the current working directory (possibly
    ///   including symbolic link components), and
    /// - there is no dot (`.`) or dot-dot (`..`) component in the pathname.
    #[must_use]
    pub fn get_pwd_if_correct(&self) -> Option<&str> {
        match self.variables.get("PWD") {
            Some(Variable {
                value: Some(Scalar(pwd)),
                ..
            }) if Path::new(pwd).is_absolute() && !has_dot_or_dot_dot(pwd) => {
                const AT_FLAGS: AtFlags = AtFlags::empty();
                let cstr_pwd = CString::new(pwd.as_bytes()).ok()?;
                let s1 = self.system.fstatat(AT_FDCWD, &cstr_pwd, AT_FLAGS).ok()?;
                let dot = CStr::from_bytes_with_nul(b".\0").unwrap();
                let s2 = self.system.fstatat(AT_FDCWD, dot, AT_FLAGS).ok()?;
                same_files(&s1, &s2).then_some(pwd)
            }
            _ => None,
        }
    }

    /// Tests if the `$PWD` variable is correct.
    #[inline]
    #[must_use]
    fn has_correct_pwd(&self) -> bool {
        self.get_pwd_if_correct().is_some()
    }

    /// Updates the `$PWD` variable with the current working directory.
    ///
    /// If the value of `$PWD` is [correct](Self::get_pwd_if_correct), this
    /// function does not modify it. Otherwise, this function sets the value to
    /// `self.system.getcwd()`.
    ///
    /// This function is meant for initializing the `$PWD` variable when the
    /// shell starts.
    pub fn prepare_pwd(&mut self) -> Result<(), PreparePwdError> {
        if !self.has_correct_pwd() {
            let dir = self
                .system
                .getcwd()?
                .into_os_string()
                .into_string()
                .map_err(|_| nix::Error::EILSEQ)?;
            self.variables
                .assign(Global, "PWD".to_string(), Variable::new(dir))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::r#virtual::FileBody;
    use crate::system::r#virtual::INode;
    use crate::variable::Value;
    use crate::VirtualSystem;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    #[test]
    fn has_dot_or_dot_dot_cases() {
        assert!(!has_dot_or_dot_dot(""));
        assert!(!has_dot_or_dot_dot("foo"));
        assert!(!has_dot_or_dot_dot(".foo"));
        assert!(!has_dot_or_dot_dot("foo.bar"));
        assert!(!has_dot_or_dot_dot("..."));
        assert!(!has_dot_or_dot_dot("/"));
        assert!(!has_dot_or_dot_dot("/bar"));
        assert!(!has_dot_or_dot_dot("/bar/baz"));

        assert!(has_dot_or_dot_dot("."));
        assert!(has_dot_or_dot_dot("/."));
        assert!(has_dot_or_dot_dot("./"));
        assert!(has_dot_or_dot_dot("/./"));
        assert!(has_dot_or_dot_dot("foo/.//bar"));

        assert!(has_dot_or_dot_dot(".."));
        assert!(has_dot_or_dot_dot("/.."));
        assert!(has_dot_or_dot_dot("../"));
        assert!(has_dot_or_dot_dot("/../"));
        assert!(has_dot_or_dot_dot("/foo//../bar"));
    }

    fn env_with_symlink_to_dir() -> Env {
        let mut system = Box::new(VirtualSystem::new());
        let mut state = system.state.borrow_mut();
        state
            .file_system
            .save(
                "/foo/bar/dir",
                Rc::new(RefCell::new(INode {
                    body: FileBody::Directory {
                        files: Default::default(),
                    },
                    permissions: Default::default(),
                })),
            )
            .unwrap();
        state
            .file_system
            .save(
                "/foo/link",
                Rc::new(RefCell::new(INode {
                    body: FileBody::Symlink {
                        target: "bar/dir".into(),
                    },
                    permissions: Default::default(),
                })),
            )
            .unwrap();
        drop(state);
        system.current_process_mut().cwd = PathBuf::from("/foo/bar/dir");
        Env::with_system(system)
    }

    #[test]
    fn prepare_pwd_no_value() {
        let mut env = env_with_symlink_to_dir();

        let result = env.prepare_pwd();
        assert_eq!(result, Ok(()));
        let pwd = env.variables.get("PWD").unwrap();
        assert_eq!(pwd.value, Some(Value::scalar("/foo/bar/dir")));
    }

    #[test]
    fn prepare_pwd_with_correct_path() {
        let mut env = env_with_symlink_to_dir();
        env.variables
            .assign(Global, "PWD".to_string(), Variable::new("/foo/link"))
            .unwrap();

        let result = env.prepare_pwd();
        assert_eq!(result, Ok(()));
        let pwd = env.variables.get("PWD").unwrap();
        assert_eq!(pwd.value, Some(Value::scalar("/foo/link")));
    }

    #[test]
    fn prepare_pwd_with_dot() {
        let mut env = env_with_symlink_to_dir();
        env.variables
            .assign(Global, "PWD".to_string(), Variable::new("/foo/./link"))
            .unwrap();

        let result = env.prepare_pwd();
        assert_eq!(result, Ok(()));
        let pwd = env.variables.get("PWD").unwrap();
        assert_eq!(pwd.value, Some(Value::scalar("/foo/bar/dir")));
    }

    #[test]
    fn prepare_pwd_with_dot_dot() {
        let mut env = env_with_symlink_to_dir();
        env.variables
            .assign(Global, "PWD".to_string(), Variable::new("/foo/./link"))
            .unwrap();

        let result = env.prepare_pwd();
        assert_eq!(result, Ok(()));
        let pwd = env.variables.get("PWD").unwrap();
        assert_eq!(pwd.value, Some(Value::scalar("/foo/bar/dir")));
    }

    #[test]
    fn prepare_pwd_with_wrong_path() {
        let mut env = env_with_symlink_to_dir();
        env.variables
            .assign(Global, "PWD".to_string(), Variable::new("/foo/bar"))
            .unwrap();

        let result = env.prepare_pwd();
        assert_eq!(result, Ok(()));
        let pwd = env.variables.get("PWD").unwrap();
        assert_eq!(pwd.value, Some(Value::scalar("/foo/bar/dir")));
    }

    #[test]
    fn prepare_pwd_with_non_absolute_path() {
        let mut system = Box::new(VirtualSystem::new());
        let mut state = system.state.borrow_mut();
        state
            .file_system
            .save(
                "/link",
                Rc::new(RefCell::new(INode {
                    body: FileBody::Symlink { target: ".".into() },
                    permissions: Default::default(),
                })),
            )
            .unwrap();
        drop(state);
        system.current_process_mut().cwd = PathBuf::from("/");

        let mut env = Env::with_system(system);
        env.variables
            .assign(Global, "PWD".to_string(), Variable::new("link"))
            .unwrap();

        let result = env.prepare_pwd();
        assert_eq!(result, Ok(()));
        let pwd = env.variables.get("PWD").unwrap();
        assert_eq!(pwd.value, Some(Value::scalar("/")));
    }
}
