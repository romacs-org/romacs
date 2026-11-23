//! Forward lexical transitions shared by partial parsing and list motion.
//! The consumers decide when to transition and keep their own stop state.
use super::{
    SCHARQUOTE, SCLOSE, SCOMMENT, SCOMMENT_FENCE, SESCAPE, SMATH, SOPEN, SQUOTE, SSTRING,
    SSTRING_FENCE, SSYMBOL, SWORD, comment_style, comstart_first, comstart_second, nested, prefix,
};

pub(super) struct CommentStart {
    pub(super) style: i32,
    pub(super) nested: bool,
}

pub(super) fn comment_pair(first: i32, second: i32) -> Option<CommentStart> {
    (comstart_first(first) && comstart_second(second)).then(|| CommentStart {
        style: comment_style(second, first),
        nested: nested(first) || nested(second),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Token {
    Other,
    Escape,
    Word,
    Open,
    Close,
    Math,
    Comment,
    CommentFence,
    String,
    StringFence,
}

/// Dispatch after checking for a two-character comment opener. CODE can
/// be promoted to a comment or overridden by multibyte syntax options.
pub(super) fn token(syntax: i32, code: i32) -> Token {
    if prefix(syntax) {
        return Token::Other;
    }
    match code {
        SESCAPE | SCHARQUOTE => Token::Escape,
        SWORD | SSYMBOL => Token::Word,
        SOPEN => Token::Open,
        SCLOSE => Token::Close,
        SMATH => Token::Math,
        SCOMMENT => Token::Comment,
        SCOMMENT_FENCE => Token::CommentFence,
        SSTRING => Token::String,
        SSTRING_FENCE => Token::StringFence,
        _ => Token::Other,
    }
}

pub(super) enum Part {
    Body,
    Escape,
    End,
}

pub(super) fn symbol_part(code: i32) -> Part {
    match code {
        SWORD | SSYMBOL | SQUOTE => Part::Body,
        SESCAPE | SCHARQUOTE => Part::Escape,
        _ => Part::End,
    }
}

pub(super) fn string_part(character: u32, code: i32, terminator: u32, fenced: bool) -> Part {
    match code {
        SSTRING if !fenced && character == terminator => Part::End,
        SSTRING_FENCE if fenced => Part::End,
        SESCAPE | SCHARQUOTE => Part::Escape,
        _ => Part::Body,
    }
}
