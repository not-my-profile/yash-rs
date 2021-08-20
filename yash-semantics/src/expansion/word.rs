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

//! Initial expansion of word.

use super::AttrField;
use super::Env;
use super::Expand;
use super::ExpandToField;
use super::Expander;
use super::Expansion;
use super::Origin;
use super::Result;
use async_trait::async_trait;
use yash_syntax::syntax::Word;
use yash_syntax::syntax::WordUnit;

#[async_trait(?Send)]
impl Expand for WordUnit {
    async fn expand<E: Env>(&self, e: &mut Expander<'_, E>) -> Result {
        use WordUnit::*;
        match self {
            Unquoted(text_unit) => text_unit.expand(e).await,
            // TODO Expand Tilde correctly
            // TODO Expand SingleQuote correctly
            // TODO Expand DoubleQuote correctly
            _ => {
                e.push_str(&self.to_string(), Origin::Literal, false, false);
                Ok(())
            }
        }
    }
}

#[async_trait(?Send)]
impl Expand for Word {
    async fn expand<E: Env>(&self, e: &mut Expander<'_, E>) -> Result {
        self.units.expand(e).await
    }
}

#[async_trait(?Send)]
impl ExpandToField for Word {
    async fn expand_to_field<E: Env>(&self, env: &mut E) -> Result<AttrField> {
        let mut chars = Vec::new();
        self.units
            .expand(&mut Expander::new(env, &mut chars))
            .await?;
        let origin = self.location.clone();
        Ok(AttrField { chars, origin })
    }

    async fn expand_to_fields<E: Env>(&self, env: &mut E) -> Result<Vec<AttrField>> {
        let mut fields = Vec::new();
        self.units
            .expand(&mut Expander::new(env, &mut fields))
            .await?;
        Ok(fields
            .into_iter()
            .map(|chars| AttrField {
                chars,
                origin: self.location.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::AttrChar;
    use super::*;
    use futures_executor::block_on;

    #[derive(Debug)]
    struct NullEnv;

    impl Env for NullEnv {}

    #[test]
    fn unquoted_expand() {
        let mut field = Vec::<AttrChar>::default();
        let mut env = NullEnv;
        let mut e = Expander::new(&mut env, &mut field);
        let u: WordUnit = "x".parse().unwrap();
        block_on(u.expand(&mut e)).unwrap();
        assert_eq!(
            field,
            [AttrChar {
                value: 'x',
                origin: Origin::Literal,
                is_quoted: false,
                is_quoting: false
            }]
        );
    }

    #[test]
    fn word_expand() {
        let mut field = Vec::<AttrChar>::default();
        let mut env = NullEnv;
        let mut e = Expander::new(&mut env, &mut field);
        let w: Word = "xyz".parse().unwrap();
        block_on(w.expand(&mut e)).unwrap();
        assert_eq!(
            field,
            [
                AttrChar {
                    value: 'x',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                },
                AttrChar {
                    value: 'y',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                },
                AttrChar {
                    value: 'z',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                }
            ]
        );
    }

    #[test]
    fn word_expand_to_field() {
        let mut env = NullEnv;
        let w: Word = "abc".parse().unwrap();
        let result = block_on(w.expand_to_field(&mut env)).unwrap();
        assert_eq!(
            result.chars,
            [
                AttrChar {
                    value: 'a',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                },
                AttrChar {
                    value: 'b',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                },
                AttrChar {
                    value: 'c',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                }
            ]
        );
        assert_eq!(result.origin, w.location);
    }

    #[test]
    fn word_expand_to_fields() {
        let mut env = NullEnv;
        let w: Word = "abc".parse().unwrap();
        let result = block_on(w.expand_to_fields(&mut env)).unwrap();
        assert_eq!(result.len(), 1, "{:?}", result);
        assert_eq!(
            result[0].chars,
            [
                AttrChar {
                    value: 'a',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                },
                AttrChar {
                    value: 'b',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                },
                AttrChar {
                    value: 'c',
                    origin: Origin::Literal,
                    is_quoted: false,
                    is_quoting: false
                }
            ]
        );
        assert_eq!(result[0].origin, w.location);
        // TODO Test with a word that expands to more than one field
    }
}
