// This file is part of yash, an extended POSIX shell.
// Copyright (C) 2020 WATANABE Yuki
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

//! Lexical analyzer.
//!
//! TODO Elaborate

mod core;
// mod heredoc; // See below
mod misc;
mod op;
mod tilde;

pub use self::core::*;
pub use self::heredoc::PartialHereDoc;
pub use self::op::is_operator_char;

use self::keyword::Keyword;
use crate::parser::core::Error;
use crate::parser::core::Result;
use crate::parser::core::SyntaxError;
use crate::source::Location;
use crate::source::SourceChar;
use crate::syntax::*;
use std::convert::TryFrom;
use std::future::Future;
use std::pin::Pin;

/// Tests whether the given character is a token delimiter.
///
/// A character is a token delimiter if it is either a whitespace or [operator](is_operator_char).
pub fn is_token_delimiter_char(c: char) -> bool {
    is_operator_char(c) || is_blank(c)
}

impl Lexer {
    /// Parses a command substitution of the form `$(...)`.
    ///
    /// The initial `$` must have been consumed before calling this function.
    /// In this function, the next character is examined to see if it begins a
    /// command substitution. If it is `(`, the following characters are parsed
    /// as commands to find a matching `)`, which will be consumed before this
    /// function returns. Otherwise, no characters are consumed and the return
    /// value is `Ok(None)`.
    ///
    /// `opening_location` should be the location of the initial `$`. It is used
    /// to construct the result, but this function does not check if it actually
    /// is a location of `$`.
    ///
    /// This function does not consume line continuations between `$` and `(`.
    /// Line continuations should have been consumed beforehand.
    pub async fn command_substitution(
        &mut self,
        opening_location: Location,
    ) -> Result<Option<TextUnit>> {
        if !self.skip_if(|c| c == '(').await? {
            return Ok(None);
        }

        let content = self.inner_program_boxed().await?;

        if !self.skip_if(|c| c == ')').await? {
            // TODO Return a better error depending on the token id of the next token
            let cause = SyntaxError::UnclosedCommandSubstitution { opening_location }.into();
            let location = self.location().await?.clone();
            return Err(Error { cause, location });
        }

        let location = opening_location;
        Ok(Some(TextUnit::CommandSubst { content, location }))
    }

    /// Parses an arithmetic expansion.
    ///
    /// The initial `$` must have been consumed before calling this function.
    /// In this function, the next two characters are examined to see if they
    /// begin an arithmetic expansion. If the characters are `((`, then the
    /// arithmetic expansion is parsed, in which case this function consumes up
    /// to the closing `))` (inclusive). Otherwise, no characters are consumed
    /// and the return value is `Ok(Err(opening_location))`.
    ///
    /// The `location` parameter should be the location of the initial `$`. It
    /// is used to construct the result, but this function does not check if it
    /// actually is a location of `$`.
    ///
    /// This function does not consume line continuations between `$` and `(`.
    /// Line continuations should have been consumed beforehand.
    pub async fn arithmetic_expansion(
        &mut self,
        location: Location,
    ) -> Result<std::result::Result<TextUnit, Location>> {
        let index = self.index();

        // Part 1: Parse `((`
        if !self.skip_if(|c| c == '(').await? {
            return Ok(Err(location));
        }
        self.line_continuations().await?;
        if !self.skip_if(|c| c == '(').await? {
            self.rewind(index);
            return Ok(Err(location));
        }

        // Part 2: Parse the content
        let is_delimiter = |c| c == ')';
        let is_escapable = |c| matches!(c, '$' | '`' | '\\');
        // Boxing needed for recursion
        let content: Pin<Box<dyn Future<Output = Result<Text>>>> =
            Box::pin(self.text_with_parentheses(is_delimiter, is_escapable));
        let content = content.await?;

        // Part 3: Parse `))`
        match self.peek_char().await? {
            Some(sc) if sc.value == ')' => self.consume_char(),
            Some(_) => unreachable!(),
            None => {
                let opening_location = location;
                let cause = SyntaxError::UnclosedArith { opening_location }.into();
                let location = self.location().await?.clone();
                return Err(Error { cause, location });
            }
        }
        self.line_continuations().await?;
        match self.peek_char().await? {
            Some(sc) if sc.value == ')' => self.consume_char(),
            Some(_) => {
                self.rewind(index);
                return Ok(Err(location));
            }
            None => {
                let opening_location = location;
                let cause = SyntaxError::UnclosedArith { opening_location }.into();
                let location = self.location().await?.clone();
                return Err(Error { cause, location });
            }
        }

        Ok(Ok(TextUnit::Arith { content, location }))
    }

    /// Parses a text unit that starts with `$`.
    ///
    /// If the next character is `$`, a parameter expansion, command
    /// substitution, or arithmetic expansion is parsed. Otherwise, no
    /// characters are consumed and the return value is `Ok(None)`.
    pub async fn dollar_unit(&mut self) -> Result<Option<TextUnit>> {
        let index = self.index();
        let location = match self.consume_char_if(|c| c == '$').await? {
            None => return Ok(None),
            Some(c) => c.location.clone(),
        };

        // TODO line continuations following $
        // TODO braced parameter expansion
        // TODO non-braced parameter expansion

        let location = match self.arithmetic_expansion(location).await? {
            Ok(result) => return Ok(Some(result)),
            Err(location) => location,
        };

        if let Some(result) = self.command_substitution(location).await? {
            return Ok(Some(result));
        }

        self.rewind(index);
        Ok(None)
    }

    /// Parses a backquote unit, possibly preceded by line continuations.
    async fn backquote_unit(
        &mut self,
        double_quote_escapable: bool,
    ) -> Result<Option<BackquoteUnit>> {
        self.line_continuations().await?;

        if self.skip_if(|c| c == '\\').await? {
            let is_escapable =
                |c| matches!(c, '$' | '`' | '\\') || c == '"' && double_quote_escapable;
            if let Some(c) = self.consume_char_if(is_escapable).await? {
                return Ok(Some(BackquoteUnit::Backslashed(c.value)));
            } else {
                return Ok(Some(BackquoteUnit::Literal('\\')));
            }
        }

        if let Some(c) = self.consume_char_if(|c| c != '`').await? {
            return Ok(Some(BackquoteUnit::Literal(c.value)));
        }

        Ok(None)
    }

    /// Parses a command substitution of the form `` `...` ``.
    ///
    /// If the next character is a backquote, the command substitution is parsed
    /// up to the closing backquote (inclusive). It is a syntax error if there is
    /// no closing backquote.
    ///
    /// Between the backquotes, only backslashes can have special meanings. A
    /// backslash followed by a newline is parsed as line continuation. A
    /// backslash is an escape character if it precedes a dollar, backquote, or
    /// another backslash. If `double_quote_escapable` is true, double quotes can
    /// also be backslash-escaped.
    pub async fn backquote(&mut self, double_quote_escapable: bool) -> Result<Option<TextUnit>> {
        let location = match self.consume_char_if(|c| c == '`').await? {
            None => return Ok(None),
            Some(c) => c.location.clone(),
        };

        let mut content = Vec::new();
        while let Some(unit) = self.backquote_unit(double_quote_escapable).await? {
            content.push(unit);
        }

        if self.skip_if(|c| c == '`').await? {
            Ok(Some(TextUnit::Backquote { content, location }))
        } else {
            let opening_location = location;
            let cause = SyntaxError::UnclosedBackquote { opening_location }.into();
            let location = self.location().await?.clone();
            Err(Error { cause, location })
        }
    }

    /// Parses a [`TextUnit`].
    ///
    /// This function parses a literal character, backslash-escaped character,
    /// [dollar unit](Self::dollar_unit), or [backquote](Self::backquote),
    /// optionally preceded by line continuations.
    ///
    /// `is_delimiter` is a function that decides if a character is a delimiter.
    /// An unquoted character is parsed only if `is_delimiter` returns false for
    /// it.
    ///
    /// `is_escapable` decides if a character can be escaped by a backslash. When
    /// `is_escapable` returns false, the preceding backslash is considered
    /// literal.
    ///
    /// If the text unit is a backquote, treatment of `\"` inside the backquote
    /// depends on `is_escapable('_')`. If it is false, the text unit is in a
    /// double-quoted context and `\"` is an escaped double-quote. Otherwise, the
    /// text unit is in an unquoted context and `\"` is treated literally.
    pub async fn text_unit<F, G>(
        &mut self,
        is_delimiter: F,
        is_escapable: G,
    ) -> Result<Option<TextUnit>>
    where
        F: FnOnce(char) -> bool,
        G: FnOnce(char) -> bool,
    {
        self.line_continuations().await?;

        if self.skip_if(|c| c == '\\').await? {
            if let Some(c) = self.consume_char_if(is_escapable).await? {
                return Ok(Some(Backslashed(c.value)));
            } else {
                return Ok(Some(Literal('\\')));
            }
        }

        if let Some(u) = self.dollar_unit().await? {
            return Ok(Some(u));
        }

        if let Some(u) = self.backquote(!is_escapable('_')).await? {
            return Ok(Some(u));
        }

        if let Some(sc) = self.consume_char_if(|c| !is_delimiter(c)).await? {
            return Ok(Some(Literal(sc.value)));
        }

        Ok(None)
    }

    /// Parses a text, i.e., a (possibly empty) sequence of [`TextUnit`]s.
    ///
    /// `is_delimiter` tests if an unquoted character is a delimiter. When
    /// `is_delimiter` returns true, the parser ends parsing and returns the text
    /// up to the character as a result.
    ///
    /// `is_escapable` tests if a backslash can escape a character. When the
    /// parser founds an unquoted backslash, the next character is passed to
    /// `is_escapable`. If `is_escapable` returns true, the backslash is treated
    /// as a valid escape (`TextUnit::Backslashed`). Otherwise, it ia a
    /// literal (`TextUnit::Literal`).
    ///
    /// `is_escapable` also affects escaping of double-quotes inside backquotes.
    /// See [`text_unit`](Self::text_unit) for details.
    pub async fn text<F, G>(&mut self, mut is_delimiter: F, mut is_escapable: G) -> Result<Text>
    where
        F: FnMut(char) -> bool,
        G: FnMut(char) -> bool,
    {
        let mut units = vec![];

        while let Some(unit) = self.text_unit(&mut is_delimiter, &mut is_escapable).await? {
            units.push(unit);
        }

        Ok(Text(units))
    }

    /// Parses a text that may contain nested parentheses.
    ///
    /// This function works similarly to [`text`](Self::text). However, if an
    /// unquoted `(` is found in the text, all text units are parsed up to the
    /// next matching unquoted `)`. Inside the parentheses, the `is_delimiter`
    /// function is ignored and all non-special characters are parsed as literal
    /// word units. After finding the `)`, this function continues parsing to
    /// find a delimiter (as per `is_delimiter`) or another parentheses.
    ///
    /// Nested parentheses are supported: the number of `(`s and `)`s must
    /// match. In other words, the final delimiter is recognized only outside
    /// outermost parentheses.
    pub async fn text_with_parentheses<F, G>(
        &mut self,
        mut is_delimiter: F,
        mut is_escapable: G,
    ) -> Result<Text>
    where
        F: FnMut(char) -> bool,
        G: FnMut(char) -> bool,
    {
        let mut units = Vec::new();
        let mut open_paren_locations = Vec::new();
        loop {
            let is_delimiter_or_paren = |c| {
                if c == '(' {
                    return true;
                }
                if open_paren_locations.is_empty() {
                    is_delimiter(c)
                } else {
                    c == ')'
                }
            };
            let next_units = self.text(is_delimiter_or_paren, &mut is_escapable).await?.0;
            units.extend(next_units);
            if let Some(sc) = self.consume_char_if(|c| c == '(').await? {
                units.push(Literal('('));
                open_paren_locations.push(sc.location.clone());
            } else if let Some(opening_location) = open_paren_locations.pop() {
                if self.skip_if(|c| c == ')').await? {
                    units.push(Literal(')'));
                } else {
                    let cause = SyntaxError::UnclosedParen { opening_location }.into();
                    let location = self.location().await?.clone();
                    return Err(Error { cause, location });
                }
            } else {
                break;
            }
        }
        Ok(Text(units))
    }

    /// Parses a single-quoted string.
    ///
    /// The opening `'` must have been consumed before calling this function.
    /// The closing `'` is consumed in this function.
    ///
    /// `opening_location` should be the location of the opening `'`. It is used
    /// to construct an error value, but this function does not check if it
    /// actually is a location of `'`.
    async fn single_quote(&mut self, opening_location: Location) -> Result<WordUnit> {
        let mut content = String::new();
        loop {
            match self.consume_char_if(|_| true).await? {
                Some(&SourceChar { value: '\'', .. }) => return Ok(SingleQuote(content)),
                Some(&SourceChar { value, .. }) => content.push(value),
                None => {
                    let cause = SyntaxError::UnclosedSingleQuote { opening_location }.into();
                    let location = self.location().await?.clone();
                    return Err(Error { cause, location });
                }
            }
        }
    }

    /// Parses a double-quoted string.
    ///
    /// The opening `"` must have been consumed before calling this function.
    /// The closing `"` is consumed in this function.
    ///
    /// `opening_location` should be the location of the opening `"`. It is used
    /// to construct an error value, but this function does not check if it
    /// actually is a location of `"`.
    async fn double_quote(&mut self, opening_location: Location) -> Result<WordUnit> {
        fn is_delimiter(c: char) -> bool {
            c == '"'
        }
        fn is_escapable(c: char) -> bool {
            matches!(c, '$' | '`' | '"' | '\\')
        }

        let content = self.text(is_delimiter, is_escapable).await?;

        if self.skip_if(|c| c == '"').await? {
            Ok(DoubleQuote(content))
        } else {
            let cause = SyntaxError::UnclosedDoubleQuote { opening_location }.into();
            let location = self.location().await?.clone();
            Err(Error { cause, location })
        }
    }

    /// Parses a word unit.
    ///
    /// `is_delimiter` is a function that decides a character is a delimiter. An
    /// unquoted character is parsed only if `is_delimiter` returns false for it.
    pub async fn word_unit<F>(&mut self, is_delimiter: F) -> Result<Option<WordUnit>>
    where
        F: FnOnce(char) -> bool,
    {
        // TODO Parse line continuations before the word unit
        // TODO Parse other types of word units
        match self.consume_char_if(|c| c == '\'' || c == '"').await? {
            None => Ok(self.text_unit(is_delimiter, |_| true).await?.map(Unquoted)),
            Some(sc) => {
                let location = sc.location.clone();
                match sc.value {
                    '\'' => self.single_quote(location).await.map(Some),
                    '"' => self.double_quote(location).await.map(Some),
                    _ => unreachable!(),
                }
            }
        }
    }

    /// Parses a word token.
    ///
    /// `is_delimiter` is a function that decides which character is a delimiter.
    /// The word ends when an unquoted delimiter is found. To parse a normal word
    /// token, you should pass [`is_token_delimiter_char`] as `is_delimiter`.
    /// Other functions can be passed to parse a word that ends with different
    /// delimiters.
    ///
    /// This function does not parse any tilde expansions in the word.
    /// To parse them, you need to call [`Word::parse_tilde_front`] or
    /// [`Word::parse_tilde_everywhere`] on the resultant word.
    pub async fn word<F>(&mut self, mut is_delimiter: F) -> Result<Word>
    where
        F: FnMut(char) -> bool,
    {
        let location = self.location().await?.clone();
        let mut units = vec![];
        while let Some(unit) = self.word_unit(&mut is_delimiter).await? {
            units.push(unit)
        }
        Ok(Word { units, location })
    }

    /// Determines the token ID for the word.
    ///
    /// This is a helper function used by [`Lexer::token`] and does not support
    /// operators.
    async fn token_id(&mut self, word: &Word) -> Result<TokenId> {
        if word.units.is_empty() {
            return Ok(TokenId::EndOfInput);
        }

        if let Some(literal) = word.to_string_if_literal() {
            if let Ok(keyword) = Keyword::try_from(literal.as_str()) {
                return Ok(TokenId::Token(Some(keyword)));
            }

            if literal.chars().all(|c| c.is_ascii_digit()) {
                // TODO Do we need to handle line continuations?
                if let Some(next) = self.peek_char().await? {
                    if next.value == '<' || next.value == '>' {
                        return Ok(TokenId::IoNumber);
                    }
                }
            }
        }

        Ok(TokenId::Token(None))
    }

    /// Parses a token.
    ///
    /// If there is no more token that can be parsed, the result is a token with an empty word and
    /// [`EndOfInput`](TokenId::EndOfInput) token identifier.
    pub async fn token(&mut self) -> Result<Token> {
        if let Some(op) = self.operator().await? {
            return Ok(op);
        }

        let index = self.index();
        let mut word = self.word(is_token_delimiter_char).await?;
        word.parse_tilde_front();
        let id = self.token_id(&word).await?;
        Ok(Token { word, id, index })
    }
}

// This is here to get better order of Lexer members in the doc.
mod heredoc;
pub mod keyword;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::core::ErrorCause;
    use crate::source::Source;
    use futures::executor::block_on;

    #[test]
    fn lexer_command_substitution_success() {
        let mut lexer = Lexer::with_source(Source::Unknown, "( foo bar )baz");
        let location = Location::dummy("X".to_string());

        let result = block_on(lexer.command_substitution(location))
            .unwrap()
            .unwrap();
        if let TextUnit::CommandSubst { location, content } = result {
            assert_eq!(location.line.value, "X");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 1);
            assert_eq!(content, " foo bar ");
        } else {
            panic!("unexpected result {:?}", result);
        }

        let next = block_on(lexer.location()).unwrap();
        assert_eq!(next.line.value, "( foo bar )baz");
        assert_eq!(next.line.number.get(), 1);
        assert_eq!(next.line.source, Source::Unknown);
        assert_eq!(next.column.get(), 12);
    }

    #[test]
    fn lexer_command_substitution_none() {
        let mut lexer = Lexer::with_source(Source::Unknown, " foo bar )baz");
        let location = Location::dummy("Y".to_string());

        let result = block_on(lexer.command_substitution(location)).unwrap();
        assert_eq!(result, None);

        let next = block_on(lexer.location()).unwrap();
        assert_eq!(next.line.value, " foo bar )baz");
        assert_eq!(next.line.number.get(), 1);
        assert_eq!(next.line.source, Source::Unknown);
        assert_eq!(next.column.get(), 1);
    }

    #[test]
    fn lexer_command_substitution_unclosed() {
        let mut lexer = Lexer::with_source(Source::Unknown, "( foo bar baz");
        let location = Location::dummy("Z".to_string());

        let e = block_on(lexer.command_substitution(location)).unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedCommandSubstitution { opening_location }) =
            e.cause
        {
            assert_eq!(opening_location.line.value, "Z");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 1);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "( foo bar baz");
        assert_eq!(e.location.line.number.get(), 1);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 14);
    }

    #[test]
    fn lexer_arithmetic_expansion_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "(());");
        let location = Location::dummy("X".to_string());

        let result = block_on(lexer.arithmetic_expansion(location))
            .unwrap()
            .unwrap();
        if let TextUnit::Arith { content, location } = result {
            assert_eq!(content.0, []);
            assert_eq!(location.line.value, "X");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not an arithmetic expansion: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ';');
    }

    #[test]
    fn lexer_arithmetic_expansion_none() {
        let mut lexer = Lexer::with_source(Source::Unknown, "( foo bar )baz");
        let location = Location::dummy("Y".to_string());

        let location = block_on(lexer.arithmetic_expansion(location))
            .unwrap()
            .unwrap_err();
        assert_eq!(location.line.value, "Y");
        assert_eq!(location.line.number.get(), 1);
        assert_eq!(location.line.source, Source::Unknown);
        assert_eq!(location.column.get(), 1);

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, '(');
    }

    #[test]
    fn lexer_arithmetic_expansion_line_continuations() {
        let mut lexer = Lexer::with_source(Source::Unknown, "(\\\n\\\n(\\\n)\\\n\\\n);");
        let location = Location::dummy("X".to_string());

        let result = block_on(lexer.arithmetic_expansion(location))
            .unwrap()
            .unwrap();
        if let TextUnit::Arith { content, location } = result {
            assert_eq!(content.0, []);
            assert_eq!(location.line.value, "X");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not an arithmetic expansion: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ';');
    }

    #[test]
    fn lexer_arithmetic_expansion_escapes() {
        let mut lexer = Lexer::with_source(Source::Unknown, r#"((\\\"\`\$));"#);
        let location = Location::dummy("X".to_string());

        let result = block_on(lexer.arithmetic_expansion(location))
            .unwrap()
            .unwrap();
        if let TextUnit::Arith { content, location } = result {
            assert_eq!(
                content.0,
                [
                    Backslashed('\\'),
                    Literal('\\'),
                    Literal('"'),
                    Backslashed('`'),
                    Backslashed('$')
                ]
            );
            assert_eq!(location.line.value, "X");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not an arithmetic expansion: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ';');
    }

    #[test]
    fn lexer_arithmetic_expansion_unclosed_first() {
        let mut lexer = Lexer::with_source(Source::Unknown, "((1");
        let location = Location::dummy("Z".to_string());

        let e = block_on(lexer.arithmetic_expansion(location)).unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedArith { opening_location }) = e.cause {
            assert_eq!(opening_location.line.value, "Z");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 1);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "((1");
        assert_eq!(e.location.line.number.get(), 1);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 4);
    }

    #[test]
    fn lexer_arithmetic_expansion_unclosed_second() {
        let mut lexer = Lexer::with_source(Source::Unknown, "((1)");
        let location = Location::dummy("Z".to_string());

        let e = block_on(lexer.arithmetic_expansion(location)).unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedArith { opening_location }) = e.cause {
            assert_eq!(opening_location.line.value, "Z");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 1);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "((1)");
        assert_eq!(e.location.line.number.get(), 1);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 5);
    }

    #[test]
    fn lexer_arithmetic_expansion_unclosed_but_maybe_command_substitution() {
        let mut lexer = Lexer::with_source(Source::Unknown, "((1) ");
        let location = Location::dummy("Z".to_string());

        let location = block_on(lexer.arithmetic_expansion(location))
            .unwrap()
            .unwrap_err();
        assert_eq!(location.line.value, "Z");
        assert_eq!(location.line.number.get(), 1);
        assert_eq!(location.line.source, Source::Unknown);
        assert_eq!(location.column.get(), 1);

        assert_eq!(lexer.index(), 0);
    }

    #[test]
    fn lexer_dollar_unit_no_dollar() {
        let mut lexer = Lexer::with_source(Source::Unknown, "foo");
        let result = block_on(lexer.dollar_unit()).unwrap();
        assert_eq!(result, None);

        let mut lexer = Lexer::with_source(Source::Unknown, "()");
        let result = block_on(lexer.dollar_unit()).unwrap();
        assert_eq!(result, None);
        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, '(');

        let mut lexer = Lexer::with_source(Source::Unknown, "");
        let result = block_on(lexer.dollar_unit()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn lexer_dollar_unit_dollar_followed_by_non_special() {
        let mut lexer = Lexer::with_source(Source::Unknown, "$;");
        let result = block_on(lexer.dollar_unit()).unwrap();
        assert_eq!(result, None);
        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, '$');

        let mut lexer = Lexer::with_source(Source::Unknown, "$&");
        let result = block_on(lexer.dollar_unit()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn lexer_dollar_unit_command_substitution() {
        let mut lexer = Lexer::with_source(Source::Unknown, "$()");
        let result = block_on(lexer.dollar_unit()).unwrap().unwrap();
        if let TextUnit::CommandSubst { location, content } = result {
            assert_eq!(location.line.value, "$()");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 1);
            assert_eq!(content, "");
        } else {
            panic!("unexpected result {:?}", result);
        }
        assert_eq!(block_on(lexer.peek_char()), Ok(None));

        let mut lexer = Lexer::with_source(Source::Unknown, "$( foo bar )");
        let result = block_on(lexer.dollar_unit()).unwrap().unwrap();
        if let TextUnit::CommandSubst { location, content } = result {
            assert_eq!(location.line.value, "$( foo bar )");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 1);
            assert_eq!(content, " foo bar ");
        } else {
            panic!("unexpected result {:?}", result);
        }
        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_dollar_unit_arithmetic_expansion() {
        let mut lexer = Lexer::with_source(Source::Unknown, "$((1))");
        let result = block_on(lexer.dollar_unit()).unwrap().unwrap();
        if let TextUnit::Arith { content, location } = result {
            assert_eq!(content, Text(vec![Literal('1')]));
            assert_eq!(location.line.value, "$((1))");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("unexpected result {:?}", result);
        }
        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_backquote_not_backquote() {
        let mut lexer = Lexer::with_source(Source::Unknown, "X");
        let result = block_on(lexer.backquote(false)).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn lexer_backquote_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "``");
        let result = block_on(lexer.backquote(false)).unwrap().unwrap();
        if let TextUnit::Backquote { content, location } = result {
            assert_eq!(content, []);
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not a backquote: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_backquote_literals() {
        let mut lexer = Lexer::with_source(Source::Unknown, "`echo`");
        let result = block_on(lexer.backquote(false)).unwrap().unwrap();
        if let TextUnit::Backquote { content, location } = result {
            assert_eq!(
                content,
                [
                    BackquoteUnit::Literal('e'),
                    BackquoteUnit::Literal('c'),
                    BackquoteUnit::Literal('h'),
                    BackquoteUnit::Literal('o')
                ]
            );
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not a backquote: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_backquote_with_escapes_double_quote_escapable() {
        let mut lexer = Lexer::with_source(Source::Unknown, r#"`a\a\$\`\\\"\'`"#);
        let result = block_on(lexer.backquote(true)).unwrap().unwrap();
        if let TextUnit::Backquote { content, location } = result {
            assert_eq!(
                content,
                [
                    BackquoteUnit::Literal('a'),
                    BackquoteUnit::Literal('\\'),
                    BackquoteUnit::Literal('a'),
                    BackquoteUnit::Backslashed('$'),
                    BackquoteUnit::Backslashed('`'),
                    BackquoteUnit::Backslashed('\\'),
                    BackquoteUnit::Backslashed('"'),
                    BackquoteUnit::Literal('\\'),
                    BackquoteUnit::Literal('\'')
                ]
            );
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not a backquote: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_backquote_with_escapes_double_quote_not_escapable() {
        let mut lexer = Lexer::with_source(Source::Unknown, r#"`a\a\$\`\\\"\'`"#);
        let result = block_on(lexer.backquote(false)).unwrap().unwrap();
        if let TextUnit::Backquote { content, location } = result {
            assert_eq!(
                content,
                [
                    BackquoteUnit::Literal('a'),
                    BackquoteUnit::Literal('\\'),
                    BackquoteUnit::Literal('a'),
                    BackquoteUnit::Backslashed('$'),
                    BackquoteUnit::Backslashed('`'),
                    BackquoteUnit::Backslashed('\\'),
                    BackquoteUnit::Literal('\\'),
                    BackquoteUnit::Literal('"'),
                    BackquoteUnit::Literal('\\'),
                    BackquoteUnit::Literal('\'')
                ]
            );
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not a backquote: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_backquote_line_continuation() {
        let mut lexer = Lexer::with_source(Source::Unknown, "`\\\na\\\n\\\nb\\\n`");
        let result = block_on(lexer.backquote(false)).unwrap().unwrap();
        if let TextUnit::Backquote { content, location } = result {
            assert_eq!(
                content,
                [BackquoteUnit::Literal('a'), BackquoteUnit::Literal('b')]
            );
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not a backquote: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_backquote_unclosed_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "`");
        let e = block_on(lexer.backquote(false)).unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedBackquote { opening_location }) = e.cause {
            assert_eq!(opening_location.line.value, "`");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 1);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "`");
        assert_eq!(e.location.line.number.get(), 1);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 2);
    }

    #[test]
    fn lexer_backquote_unclosed_nonempty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "`foo");
        let e = block_on(lexer.backquote(false)).unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedBackquote { opening_location }) = e.cause {
            assert_eq!(opening_location.line.value, "`foo");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 1);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "`foo");
        assert_eq!(e.location.line.number.get(), 1);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 5);
    }

    #[test]
    fn lexer_text_unit_literal_accepted() {
        let mut lexer = Lexer::with_source(Source::Unknown, "X");
        let mut called = false;
        let result = block_on(lexer.text_unit(
            |c| {
                called = true;
                assert_eq!(c, 'X');
                false
            },
            |c| {
                assert_eq!(c, '_');
                true
            },
        ))
        .unwrap()
        .unwrap();
        assert!(called);
        if let Literal(c) = result {
            assert_eq!(c, 'X');
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_unit_literal_rejected() {
        let mut lexer = Lexer::with_source(Source::Unknown, ";");
        let mut called = false;
        let result = block_on(lexer.text_unit(
            |c| {
                called = true;
                assert_eq!(c, ';');
                true
            },
            |c| {
                assert_eq!(c, '_');
                true
            },
        ))
        .unwrap();
        assert!(called);
        assert_eq!(result, None);

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ';');
    }

    #[test]
    fn lexer_text_unit_backslash_accepted() {
        let mut lexer = Lexer::with_source(Source::Unknown, r"\#");
        let mut called = false;
        let result = block_on(lexer.text_unit(
            |c| panic!("unexpected call to is_delimiter({:?})", c),
            |c| {
                called = true;
                assert_eq!(c, '#');
                true
            },
        ))
        .unwrap()
        .unwrap();
        assert!(called);
        assert_eq!(result, Backslashed('#'));

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_unit_backslash_eof() {
        let mut lexer = Lexer::with_source(Source::Unknown, r"\");
        let result = block_on(lexer.text_unit(
            |c| panic!("unexpected call to is_delimiter({:?})", c),
            |c| panic!("unexpected call to is_escapable({:?})", c),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(result, Literal('\\'));

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_unit_dollar() {
        let mut lexer = Lexer::with_source(Source::Unknown, "$()");
        let result = block_on(lexer.text_unit(
            |c| panic!("unexpected call to is_delimiter({:?})", c),
            |c| panic!("unexpected call to is_escapable({:?})", c),
        ))
        .unwrap()
        .unwrap();
        if let CommandSubst { content, location } = result {
            assert_eq!(content, "");
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_unit_backquote_double_quote_escapable() {
        let mut lexer = Lexer::with_source(Source::Unknown, r#"`\"`"#);
        let result = block_on(lexer.text_unit(
            |c| panic!("unexpected call to is_delimiter({:?})", c),
            |c| {
                assert_eq!(c, '_');
                false
            },
        ))
        .unwrap()
        .unwrap();
        if let Backquote { content, location } = result {
            assert_eq!(content, [BackquoteUnit::Backslashed('"')]);
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not a backquote: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_unit_backquote_double_quote_not_escapable() {
        let mut lexer = Lexer::with_source(Source::Unknown, r#"`\"`"#);
        let result = block_on(lexer.text_unit(
            |c| panic!("unexpected call to is_delimiter({:?})", c),
            |c| {
                assert_eq!(c, '_');
                true
            },
        ))
        .unwrap()
        .unwrap();
        if let Backquote { content, location } = result {
            assert_eq!(
                content,
                [BackquoteUnit::Literal('\\'), BackquoteUnit::Literal('"')]
            );
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("Not a backquote: {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_unit_line_continuations() {
        let mut lexer = Lexer::with_source(Source::Unknown, "\\\n\\\nX");
        let result = block_on(lexer.text_unit(
            |_| false,
            |c| {
                assert_eq!(c, '_');
                true
            },
        ))
        .unwrap()
        .unwrap();
        if let Literal(c) = result {
            assert_eq!(c, 'X');
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "");
        let Text(units) = block_on(lexer.text(
            |c| panic!("unexpected call to is_delimiter({:?})", c),
            |c| {
                assert_eq!(c, '_');
                true
            },
        ))
        .unwrap();
        assert_eq!(units, &[]);
    }

    #[test]
    fn lexer_text_nonempty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "abc");
        let mut called = 0;
        let Text(units) = block_on(lexer.text(
            |c| {
                assert!(
                    matches!(c, 'a' | 'b' | 'c'),
                    "unexpected call to is_delimiter({:?}), called={}",
                    c,
                    called
                );
                called += 1;
                false
            },
            |c| {
                assert_eq!(c, '_');
                true
            },
        ))
        .unwrap();
        assert_eq!(units, &[Literal('a'), Literal('b'), Literal('c')]);
        assert_eq!(called, 3);

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_delimiter() {
        let mut lexer = Lexer::with_source(Source::Unknown, "abc");
        let mut called = 0;
        let Text(units) = block_on(lexer.text(
            |c| {
                assert!(
                    matches!(c, 'a' | 'b' | 'c'),
                    "unexpected call to is_delimiter({:?}), called={}",
                    c,
                    called
                );
                called += 1;
                c == 'c'
            },
            |c| {
                assert_eq!(c, '_');
                true
            },
        ))
        .unwrap();
        assert_eq!(units, &[Literal('a'), Literal('b')]);
        assert_eq!(called, 3);

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, 'c');
    }

    #[test]
    fn lexer_text_escaping() {
        let mut lexer = Lexer::with_source(Source::Unknown, r"a\b\c");
        let mut tested_chars = String::new();
        let Text(units) = block_on(lexer.text(
            |_| false,
            |c| {
                tested_chars.push(c);
                c == 'b'
            },
        ))
        .unwrap();
        assert_eq!(
            units,
            &[Literal('a'), Backslashed('b'), Literal('\\'), Literal('c')]
        );
        assert_eq!(tested_chars, "_bc__");

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_with_parentheses_no_parentheses() {
        let mut lexer = Lexer::with_source(Source::Unknown, "abc");
        let Text(units) = block_on(lexer.text_with_parentheses(|_| false, |_| false)).unwrap();
        assert_eq!(units, &[Literal('a'), Literal('b'), Literal('c')]);

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_text_with_parentheses_nest_1() {
        let mut lexer = Lexer::with_source(Source::Unknown, "a(b)c)");
        let Text(units) =
            block_on(lexer.text_with_parentheses(|c| c == 'b' || c == ')', |_| false)).unwrap();
        assert_eq!(
            units,
            &[
                Literal('a'),
                Literal('('),
                Literal('b'),
                Literal(')'),
                Literal('c'),
            ]
        );

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ')');
    }

    #[test]
    fn lexer_text_with_parentheses_nest_1_1() {
        let mut lexer = Lexer::with_source(Source::Unknown, "ab(CD)ef(GH)ij;");
        let Text(units) = block_on(
            lexer.text_with_parentheses(|c| c.is_ascii_uppercase() || c == ';', |_| false),
        )
        .unwrap();
        assert_eq!(
            units,
            &[
                Literal('a'),
                Literal('b'),
                Literal('('),
                Literal('C'),
                Literal('D'),
                Literal(')'),
                Literal('e'),
                Literal('f'),
                Literal('('),
                Literal('G'),
                Literal('H'),
                Literal(')'),
                Literal('i'),
                Literal('j'),
            ]
        );

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ';');
    }

    #[test]
    fn lexer_text_with_parentheses_nest_3() {
        let mut lexer = Lexer::with_source(Source::Unknown, "a(B((C)D))e;");
        let Text(units) = block_on(
            lexer.text_with_parentheses(|c| c.is_ascii_uppercase() || c == ';', |_| false),
        )
        .unwrap();
        assert_eq!(
            units,
            &[
                Literal('a'),
                Literal('('),
                Literal('B'),
                Literal('('),
                Literal('('),
                Literal('C'),
                Literal(')'),
                Literal('D'),
                Literal(')'),
                Literal(')'),
                Literal('e'),
            ]
        );

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ';');
    }

    #[test]
    fn lexer_text_with_parentheses_unclosed() {
        let mut lexer = Lexer::with_source(Source::Unknown, "x(()");
        let e = block_on(lexer.text_with_parentheses(|_| false, |_| false)).unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedParen { opening_location }) = e.cause {
            assert_eq!(opening_location.line.value, "x(()");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 2);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "x(()");
        assert_eq!(e.location.line.number.get(), 1);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 5);
    }

    #[test]
    fn lexer_word_unit_unquoted() {
        let mut lexer = Lexer::with_source(Source::Unknown, "$()");
        let result =
            block_on(lexer.word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c)))
                .unwrap()
                .unwrap();
        if let Unquoted(CommandSubst { content, location }) = result {
            assert_eq!(content, "");
            assert_eq!(location.column.get(), 1);
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_word_unit_unquoted_escapes() {
        // Any characters can be escaped in this context.
        block_on(async {
            let mut lexer = Lexer::with_source(Source::Unknown, r#"\a\$\`\"\\\'\#"#);
            let result = lexer
                .word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c))
                .await;
            assert_eq!(result, Ok(Some(Unquoted(Backslashed('a')))));
            let result = lexer
                .word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c))
                .await;
            assert_eq!(result, Ok(Some(Unquoted(Backslashed('$')))));
            let result = lexer
                .word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c))
                .await;
            assert_eq!(result, Ok(Some(Unquoted(Backslashed('`')))));
            let result = lexer
                .word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c))
                .await;
            assert_eq!(result, Ok(Some(Unquoted(Backslashed('"')))));
            let result = lexer
                .word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c))
                .await;
            assert_eq!(result, Ok(Some(Unquoted(Backslashed('\\')))));
            let result = lexer
                .word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c))
                .await;
            assert_eq!(result, Ok(Some(Unquoted(Backslashed('\'')))));
            let result = lexer
                .word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c))
                .await;
            assert_eq!(result, Ok(Some(Unquoted(Backslashed('#')))));

            assert_eq!(lexer.peek_char().await, Ok(None));
        })
    }

    #[test]
    fn lexer_word_unit_single_quote_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "''");
        let result =
            block_on(lexer.word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c)))
                .unwrap()
                .unwrap();
        if let SingleQuote(content) = result {
            assert_eq!(content, "");
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_word_unit_single_quote_nonempty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "'abc\n$def\\'");
        let result =
            block_on(lexer.word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c)))
                .unwrap()
                .unwrap();
        if let SingleQuote(content) = result {
            assert_eq!(content, "abc\n$def\\");
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_word_unit_single_quote_unclosed() {
        let mut lexer = Lexer::with_source(Source::Unknown, "'abc\ndef\\");

        let e = block_on(lexer.word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c)))
            .unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedSingleQuote { opening_location }) = e.cause {
            assert_eq!(opening_location.line.value, "'abc\n");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 1);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "def\\");
        assert_eq!(e.location.line.number.get(), 2);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 5);
    }

    #[test]
    fn lexer_word_unit_double_quote_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "\"\"");
        let result =
            block_on(lexer.word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c)))
                .unwrap()
                .unwrap();
        if let DoubleQuote(Text(content)) = result {
            assert_eq!(content, []);
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_word_unit_double_quote_non_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "\"abc\"");
        let result =
            block_on(lexer.word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c)))
                .unwrap()
                .unwrap();
        if let DoubleQuote(Text(content)) = result {
            assert_eq!(content, [Literal('a'), Literal('b'), Literal('c')]);
        } else {
            panic!("unexpected result {:?}", result);
        }

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_word_unit_double_quote_escapes() {
        // Only the following can be escaped in this context: $ ` " \
        block_on(async {
            let mut lexer = Lexer::with_source(Source::Unknown, r#""\a\$\`\"\\\'\#""#);
            let result = lexer
                .word_unit(|c| match c {
                    'a' | '\'' | '#' => true,
                    _ => panic!("unexpected call to is_delimiter({:?})", c),
                })
                .await
                .unwrap()
                .unwrap();
            if let DoubleQuote(Text(ref units)) = result {
                assert_eq!(
                    units,
                    &[
                        Literal('\\'),
                        Literal('a'),
                        Backslashed('$'),
                        Backslashed('`'),
                        Backslashed('"'),
                        Backslashed('\\'),
                        Literal('\\'),
                        Literal('\''),
                        Literal('\\'),
                        Literal('#'),
                    ]
                );
            } else {
                panic!("Not a double quote: {:?}", result);
            }

            assert_eq!(lexer.peek_char().await, Ok(None));
        })
    }

    #[test]
    fn lexer_word_unit_double_quote_unclosed() {
        let mut lexer = Lexer::with_source(Source::Unknown, "\"abc\ndef");

        let e = block_on(lexer.word_unit(|c| panic!("unexpected call to is_delimiter({:?})", c)))
            .unwrap_err();
        if let ErrorCause::Syntax(SyntaxError::UnclosedDoubleQuote { opening_location }) = e.cause {
            assert_eq!(opening_location.line.value, "\"abc\n");
            assert_eq!(opening_location.line.number.get(), 1);
            assert_eq!(opening_location.line.source, Source::Unknown);
            assert_eq!(opening_location.column.get(), 1);
        } else {
            panic!("unexpected error cause {:?}", e);
        }
        assert_eq!(e.location.line.value, "def");
        assert_eq!(e.location.line.number.get(), 2);
        assert_eq!(e.location.line.source, Source::Unknown);
        assert_eq!(e.location.column.get(), 4);
    }

    #[test]
    fn lexer_word_nonempty() {
        let mut lexer = Lexer::with_source(Source::Unknown, r"0$(:)X\#");
        let word = block_on(lexer.word(|_| false)).unwrap();
        assert_eq!(word.units.len(), 4);
        assert_eq!(word.units[0], WordUnit::Unquoted(TextUnit::Literal('0')));
        if let WordUnit::Unquoted(TextUnit::CommandSubst { content, location }) = &word.units[1] {
            assert_eq!(content, ":");
            assert_eq!(location.line.value, r"0$(:)X\#");
            assert_eq!(location.line.number.get(), 1);
            assert_eq!(location.line.source, Source::Unknown);
            assert_eq!(location.column.get(), 2);
        } else {
            panic!("unexpected word unit: {:?}", word.units[1]);
        }
        assert_eq!(word.units[2], WordUnit::Unquoted(TextUnit::Literal('X')));
        assert_eq!(
            word.units[3],
            WordUnit::Unquoted(TextUnit::Backslashed('#'))
        );
        assert_eq!(word.location.line.value, r"0$(:)X\#");
        assert_eq!(word.location.line.number.get(), 1);
        assert_eq!(word.location.line.source, Source::Unknown);
        assert_eq!(word.location.column.get(), 1);

        assert_eq!(block_on(lexer.peek_char()), Ok(None));
    }

    #[test]
    fn lexer_word_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "");
        let word = block_on(lexer.word(|_| panic!("unexpected call to is_delimiter"))).unwrap();
        assert_eq!(word.units, []);
        assert_eq!(word.location.line.value, "");
        assert_eq!(word.location.line.number.get(), 1);
        assert_eq!(word.location.line.source, Source::Unknown);
        assert_eq!(word.location.column.get(), 1);
    }

    #[test]
    fn lexer_token_empty() {
        // If there's no word unit that can be parsed, it is the end of input.
        let mut lexer = Lexer::with_source(Source::Unknown, "");

        let t = block_on(lexer.token()).unwrap();
        assert_eq!(t.word.location.line.value, "");
        assert_eq!(t.word.location.line.number.get(), 1);
        assert_eq!(t.word.location.line.source, Source::Unknown);
        assert_eq!(t.word.location.column.get(), 1);
        assert_eq!(t.id, TokenId::EndOfInput);
        assert_eq!(t.index, 0);
    }

    #[test]
    fn lexer_token_non_empty() {
        let mut lexer = Lexer::with_source(Source::Unknown, "abc ");

        let t = block_on(lexer.token()).unwrap();
        assert_eq!(t.word.units.len(), 3);
        assert_eq!(t.word.units[0], WordUnit::Unquoted(TextUnit::Literal('a')));
        assert_eq!(t.word.units[1], WordUnit::Unquoted(TextUnit::Literal('b')));
        assert_eq!(t.word.units[2], WordUnit::Unquoted(TextUnit::Literal('c')));
        assert_eq!(t.word.location.line.value, "abc ");
        assert_eq!(t.word.location.line.number.get(), 1);
        assert_eq!(t.word.location.line.source, Source::Unknown);
        assert_eq!(t.word.location.column.get(), 1);
        assert_eq!(t.id, TokenId::Token(None));
        assert_eq!(t.index, 0);

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, ' ');
    }

    #[test]
    fn lexer_token_tilde() {
        let mut lexer = Lexer::with_source(Source::Unknown, "~a:~");

        let t = block_on(lexer.token()).unwrap();
        assert_eq!(
            t.word.units,
            [
                WordUnit::Tilde("a".to_string()),
                WordUnit::Unquoted(TextUnit::Literal(':')),
                WordUnit::Unquoted(TextUnit::Literal('~'))
            ]
        );
    }

    #[test]
    fn lexer_token_io_number_delimited_by_less() {
        let mut lexer = Lexer::with_source(Source::Unknown, "12<");

        let t = block_on(lexer.token()).unwrap();
        assert_eq!(t.word.units.len(), 2);
        assert_eq!(t.word.units[0], WordUnit::Unquoted(TextUnit::Literal('1')));
        assert_eq!(t.word.units[1], WordUnit::Unquoted(TextUnit::Literal('2')));
        assert_eq!(t.word.location.line.value, "12<");
        assert_eq!(t.word.location.line.number.get(), 1);
        assert_eq!(t.word.location.line.source, Source::Unknown);
        assert_eq!(t.word.location.column.get(), 1);
        assert_eq!(t.id, TokenId::IoNumber);
        assert_eq!(t.index, 0);

        assert_eq!(block_on(lexer.peek_char()).unwrap().unwrap().value, '<');
    }

    #[test]
    fn lexer_token_io_number_delimited_by_greater() {
        let mut lexer = Lexer::with_source(Source::Unknown, "0>>");

        let t = block_on(lexer.token()).unwrap();
        assert_eq!(t.word.units.len(), 1);
        assert_eq!(t.word.units[0], WordUnit::Unquoted(TextUnit::Literal('0')));
        assert_eq!(t.word.location.line.value, "0>>");
        assert_eq!(t.word.location.line.number.get(), 1);
        assert_eq!(t.word.location.line.source, Source::Unknown);
        assert_eq!(t.word.location.column.get(), 1);
        assert_eq!(t.id, TokenId::IoNumber);
        assert_eq!(t.index, 0);

        assert_eq!(block_on(lexer.location()).unwrap().column.get(), 2);
    }

    #[test]
    fn lexer_token_after_blank() {
        block_on(async {
            let mut lexer = Lexer::with_source(Source::Unknown, " a  ");

            lexer.skip_blanks().await.unwrap();
            let t = lexer.token().await.unwrap();
            assert_eq!(t.word.location.line.value, " a  ");
            assert_eq!(t.word.location.line.number.get(), 1);
            assert_eq!(t.word.location.line.source, Source::Unknown);
            assert_eq!(t.word.location.column.get(), 2);
            assert_eq!(t.id, TokenId::Token(None));
            assert_eq!(t.index, 1);

            lexer.skip_blanks().await.unwrap();
            let t = lexer.token().await.unwrap();
            assert_eq!(t.word.location.line.value, " a  ");
            assert_eq!(t.word.location.line.number.get(), 1);
            assert_eq!(t.word.location.line.source, Source::Unknown);
            assert_eq!(t.word.location.column.get(), 5);
            assert_eq!(t.id, TokenId::EndOfInput);
            assert_eq!(t.index, 4);
        });
    }
}
