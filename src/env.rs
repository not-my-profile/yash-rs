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

//! Shell execution environment.
//!
//! TODO Elaborate

use crate::alias::AliasSet;
use std::rc::Rc;

/// Alias-related part of the shell execution environment.
pub trait AliasEnv {
    /// Returns a reference to the alias set.
    fn aliases(&self) -> &Rc<AliasSet>;
    /// Returns a mutable reference to the alias set.
    fn aliases_mut(&mut self) -> &mut Rc<AliasSet>;
}

/// Minimal implementor of [`AliasEnv`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Aliases(Rc<AliasSet>);

impl AliasEnv for Aliases {
    fn aliases(&self) -> &Rc<AliasSet> {
        &self.0
    }
    fn aliases_mut(&mut self) -> &mut Rc<AliasSet> {
        &mut self.0
    }
}

/// Subset of the shell execution environment that can be implemented
/// independently of the underlying OS features.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalEnv {
    pub aliases: Aliases,
}

impl LocalEnv {
    /// Creates a new local environment.
    pub fn new() -> LocalEnv {
        let aliases = Aliases(Rc::new(AliasSet::new()));
        LocalEnv { aliases }
    }
}

impl AliasEnv for LocalEnv {
    fn aliases(&self) -> &Rc<AliasSet> {
        self.aliases.aliases()
    }
    fn aliases_mut(&mut self) -> &mut Rc<AliasSet> {
        self.aliases.aliases_mut()
    }
}

/// Whole shell execution environment.
pub trait Env: AliasEnv {}

/// Implementation of [`Env`] that is based on the state of the current process.
#[derive(Debug)]
pub struct NativeEnv {
    /// Local part of the environment.
    pub local: LocalEnv,
}

impl NativeEnv {
    /// Creates a new environment.
    ///
    /// Because `NativeEnv` is tied with the state of the current process, there
    /// should be at most one instance of `NativeEnv` in a process. Using more
    /// than one `NativeEnv` instance at the same time should be considered
    /// unsafe.
    pub fn new() -> NativeEnv {
        let local = LocalEnv::new();
        NativeEnv { local }
    }
}

impl AliasEnv for NativeEnv {
    fn aliases(&self) -> &Rc<AliasSet> {
        self.local.aliases()
    }
    fn aliases_mut(&mut self) -> &mut Rc<AliasSet> {
        self.local.aliases_mut()
    }
}

impl Env for NativeEnv {}
