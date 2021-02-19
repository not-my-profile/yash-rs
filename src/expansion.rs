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

//! Word expansions.
//!
//! TODO Elaborate

use crate::env::Env;
use crate::source::Location;
use crate::syntax::Word;

/// Errors that may happen in word expansions.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    // TODO define error variants
}

/// Result type for word expansion.
pub type Result<T = ()> = std::result::Result<T, Error>;

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

impl std::fmt::Display for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl Word {
    /// Expands `self` to fields.
    ///
    /// The result can be any number of fields.
    pub fn expand_multiple(&self, _: &mut dyn Env) -> Result<Vec<Field>> {
        // TODO Expand each word units
        Ok(vec![Field {
            value: self.to_string(),
            origin: self.location.clone(),
        }])
    }
}
