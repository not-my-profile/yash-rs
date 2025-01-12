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

//! Readonly built-in.
//!
//! TODO Elaborate

use yash_env::builtin::Result;
use yash_env::semantics::ExitStatus;
use yash_env::semantics::Field;
use yash_env::variable::ReadOnlyError;
use yash_env::variable::Scope;
use yash_env::variable::Variable;
use yash_env::Env;

// TODO Split into syntax and semantics submodules

/// Entry point for executing the `readonly` built-in
pub fn main(env: &mut Env, args: Vec<Field>) -> Result {
    // TODO support options
    // TODO print read-only variables if there are no operands

    for Field { value, origin } in args {
        if let Some(eq_index) = value.find('=') {
            let var_value = value[eq_index + 1..].to_owned();
            let var = Variable::new(var_value)
                .set_assigned_location(origin.clone())
                .make_read_only(origin);

            let mut name = value;
            name.truncate(eq_index);
            // TODO reject invalid name

            match env.assign_variable(Scope::Global, name, var) {
                Ok(_old_value) => (),
                Err(ReadOnlyError {
                    name,
                    read_only_location: _,
                    new_value: _,
                }) => {
                    // TODO Better error message
                    // TODO Use Env rather than printing directly to stderr
                    eprintln!("cannot assign to read-only variable {name}");
                    return ExitStatus::FAILURE.into();
                }
            }
        } else {
            // TODO Make an existing variable read-only or create a new value-less variable
        }
    }

    ExitStatus::SUCCESS.into()
}

#[allow(clippy::bool_assert_comparison)]
#[cfg(test)]
mod tests {
    use super::*;
    use yash_env::variable::Value;
    use yash_env::Env;

    #[test]
    fn builtin_defines_read_only_variable() {
        let mut env = Env::new_virtual();
        let args = Field::dummies(["foo=bar baz"]);
        let location = args[0].origin.clone();

        let result = main(&mut env, args);
        assert_eq!(result, Result::new(ExitStatus::SUCCESS));

        let v = env.variables.get("foo").unwrap();
        assert_eq!(v.value, Some(Value::scalar("bar baz")));
        assert_eq!(v.is_exported, false);
        assert_eq!(v.read_only_location.as_ref().unwrap(), &location);
        assert_eq!(v.last_assigned_location.as_ref().unwrap(), &location);
    }
}
