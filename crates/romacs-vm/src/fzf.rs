// SPDX-License-Identifier: MIT
//
// Ported from fzf, https://github.com/junegunn/fzf, commit
// 26ff936df038b6a62ec42cf18db27b2ea2bb8ec3: src/algo/algo.go (FuzzyMatchV1,
// FuzzyMatchV2, ExactMatchNaive, ExactMatchBoundary, PrefixMatch,
// SuffixMatch, EqualMatch), src/pattern.go (parseTerms, extendedMatch) and
// src/result.go (the score/length/index order); the tests port
// src/algo/algo_test.go.
//
// Copyright (c) 2013-2026 Junegunn Choi
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! fzf's matching for the `fzf' completion style (rlisp/romacs-fzf.el).
//!
//! Texts are character codes.  A term that ignores case is matched
//! against the candidate's case-folded codes, the caller having folded
//! the term too; character classes, and so bonuses, always come from the
//! original codes.  What Romacs leaves out of fzf: the backward
//! (`--tac`) direction, `--scheme` other than `default`, and the Latin
//! normalisation of `--literal`'s absence.  fzf's single- and
//! two-character fast paths are not ported: fzf's own tests hold them
//! equal to the general algorithm.

use std::cmp::{max, Reverse};
use std::sync::atomic::{AtomicU64, Ordering};

const SCORE_MATCH: i16 = 16;
const SCORE_GAP_START: i16 = -3;
const SCORE_GAP_EXTENSION: i16 = -1;
const BONUS_BOUNDARY: i16 = SCORE_MATCH / 2;
const BONUS_NON_WORD: i16 = SCORE_MATCH / 2;
const BONUS_CAMEL123: i16 = BONUS_BOUNDARY + SCORE_GAP_EXTENSION;
const BONUS_CONSECUTIVE: i16 = -(SCORE_GAP_START + SCORE_GAP_EXTENSION);
const BONUS_FIRST_CHAR_MULTIPLIER: i16 = 2;
const BONUS_BOUNDARY_WHITE: i16 = BONUS_BOUNDARY + 2;
const BONUS_BOUNDARY_DELIMITER: i16 = BONUS_BOUNDARY + 1;
/// fzf's slab of 16-bit cells: a longer score matrix takes FuzzyMatchV1.
const SLAB_16: usize = 100 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    White,
    NonWord,
    Delimiter,
    Lower,
    Upper,
    Letter,
    Number,
}

const INITIAL_CLASS: Class = Class::White;
const DELIMITERS: &[u32] = &['/' as u32, ',' as u32, ':' as u32, ';' as u32, '|' as u32];

pub(crate) fn class_of(code: u32) -> Class {
    if code < 128 {
        let c = code as u8;
        return match c {
            b'a'..=b'z' => Class::Lower,
            b'A'..=b'Z' => Class::Upper,
            b'0'..=b'9' => Class::Number,
            b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r' => Class::White,
            _ if DELIMITERS.contains(&code) => Class::Delimiter,
            _ => Class::NonWord,
        };
    }
    match char::from_u32(code) {
        Some(c) if c.is_lowercase() => Class::Lower,
        Some(c) if c.is_uppercase() => Class::Upper,
        Some(c) if c.is_numeric() => Class::Number,
        Some(c) if c.is_alphabetic() => Class::Letter,
        Some(c) if c.is_whitespace() => Class::White,
        _ => Class::NonWord,
    }
}

const fn bonus_for(prev: Class, class: Class) -> i16 {
    if class as u8 >= Class::NonWord as u8 {
        match prev {
            Class::White => return BONUS_BOUNDARY_WHITE,
            Class::Delimiter => return BONUS_BOUNDARY_DELIMITER,
            Class::NonWord => return BONUS_BOUNDARY,
            _ => {}
        }
    }
    let camel = matches!((prev, class), (Class::Lower, Class::Upper));
    let digit = !matches!(prev, Class::Number) && matches!(class, Class::Number);
    if camel || digit {
        return BONUS_CAMEL123;
    }
    match class {
        Class::NonWord | Class::Delimiter => BONUS_NON_WORD,
        Class::White => BONUS_BOUNDARY_WHITE,
        _ => 0,
    }
}

const CLASSES: [Class; 7] =
    [Class::White, Class::NonWord, Class::Delimiter, Class::Lower, Class::Upper, Class::Letter, Class::Number];

const BONUS: [[i16; 7]; 7] = {
    let mut matrix = [[0; 7]; 7];
    let mut i = 0;
    while i < 7 {
        let mut j = 0;
        while j < 7 {
            matrix[i][j] = bonus_for(CLASSES[i], CLASSES[j]);
            j += 1;
        }
        i += 1;
    }
    matrix
};

fn bonus(prev: Class, class: Class) -> i16 {
    BONUS[prev as usize][class as usize]
}

fn bonus_at(classes: &[Class], at: usize) -> i16 {
    if at == 0 { BONUS_BOUNDARY_WHITE } else { bonus(classes[at - 1], classes[at]) }
}

fn is_space(code: u32) -> bool {
    char::from_u32(code).is_some_and(char::is_whitespace)
}

/// A term's match: its character range, score, and, when asked for, the
/// positions of the fuzzy algorithms' matched characters.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub start: usize,
    pub end: usize,
    pub score: i32,
    pub positions: Option<Vec<usize>>,
}

fn found(start: usize, end: usize, score: i32) -> Option<Found> {
    Some(Found { start, end, score, positions: None })
}

/// Work arrays a thread reuses from candidate to candidate, as fzf's slab.
#[derive(Default)]
pub(crate) struct Scratch {
    h0: Vec<i16>,
    c0: Vec<i16>,
    b: Vec<i16>,
    f: Vec<usize>,
    h: Vec<i16>,
    c: Vec<i16>,
    classes: Vec<Class>,
}

fn reset(v: &mut Vec<i16>, n: usize) {
    v.clear();
    v.resize(n, 0);
}

/// asciiFuzzyIndex: the range that can hold PATTERN's best alignment,
/// from just before its first character's first occurrence to its last
/// character's last; None when PATTERN is not a subsequence of TEXT.
fn fuzzy_index(text: &[u32], pattern: &[u32]) -> Option<(usize, usize)> {
    let (mut from, mut first, mut last) = (0, 0, 0);
    for (i, &p) in pattern.iter().enumerate() {
        let at = from + text[from..].iter().position(|&c| c == p)?;
        if i == 0 && at > 0 {
            first = at - 1;
        }
        last = at;
        from = at + 1;
    }
    let tail = *pattern.last()?;
    let end = text[last + 1..].iter().rposition(|&c| c == tail).map_or(last + 1, |e| last + 1 + e + 1);
    Some((first, end))
}

/// FuzzyMatchV2: the alignment of PATTERN in TEXT with the best score,
/// fzf's Smith-Waterman variant that allows no omission.
pub(crate) fn fuzzy_v2(s: &mut Scratch, text: &[u32], classes: &[Class], pattern: &[u32], with_pos: bool) -> Option<Found> {
    let m = pattern.len();
    if m == 0 {
        return Some(Found { start: 0, end: 0, score: 0, positions: with_pos.then(Vec::new) });
    }
    if m > text.len() {
        return None;
    }
    if text.len() * m > SLAB_16 || m > 1000 {
        return fuzzy_v1(text, classes, pattern, with_pos);
    }
    let (min, max_idx) = fuzzy_index(text, pattern)?;
    let t = &text[min..max_idx];
    let n = t.len();
    reset(&mut s.h0, n);
    reset(&mut s.c0, n);
    reset(&mut s.b, n);
    s.f.clear();
    s.f.resize(m, 0);

    // Phase 2: the bonus at each position, and the first row.
    let (mut max_score, mut max_pos) = (0i16, 0usize);
    let (mut pidx, mut last) = (0, 0);
    let p0 = pattern[0];
    let (mut pchar, mut prev_h0, mut prev_class, mut in_gap) = (p0, 0i16, INITIAL_CLASS, false);
    for (off, &ch) in t.iter().enumerate() {
        let class = classes[min + off];
        let b = bonus(prev_class, class);
        s.b[off] = b;
        prev_class = class;
        if ch == pchar {
            if pidx < m {
                s.f[pidx] = off;
                pidx += 1;
                pchar = pattern[pidx.min(m - 1)];
            }
            last = off;
        }
        if ch == p0 {
            let score = SCORE_MATCH + b * BONUS_FIRST_CHAR_MULTIPLIER;
            s.h0[off] = score;
            s.c0[off] = 1;
            if m == 1 && score > max_score {
                (max_score, max_pos) = (score, off);
                if b >= BONUS_BOUNDARY {
                    break;
                }
            }
            in_gap = false;
        } else {
            let gap = if in_gap { SCORE_GAP_EXTENSION } else { SCORE_GAP_START };
            s.h0[off] = max(prev_h0 + gap, 0);
            s.c0[off] = 0;
            in_gap = true;
        }
        prev_h0 = s.h0[off];
    }
    if pidx != m {
        return None;
    }
    if m == 1 {
        let at = min + max_pos;
        return Some(Found { start: at, end: at + 1, score: max_score as i32, positions: with_pos.then(|| vec![at]) });
    }

    // Phase 3: the score matrix, each row from its character's first occurrence.
    let f0 = s.f[0];
    let width = last - f0 + 1;
    reset(&mut s.h, width * m);
    reset(&mut s.c, width * m);
    s.h[..width].copy_from_slice(&s.h0[f0..=last]);
    s.c[..width].copy_from_slice(&s.c0[f0..=last]);
    for pidx in 1..m {
        let (fi, pchar, row) = (s.f[pidx], pattern[pidx], pidx * width);
        let mut in_gap = false;
        s.h[row + fi - f0 - 1] = 0;
        for col in fi..=last {
            let j = col - f0;
            let gap = if in_gap { SCORE_GAP_EXTENSION } else { SCORE_GAP_START };
            let s2 = s.h[row + j - 1] + gap;
            let (mut s1, mut consecutive) = (0i16, 0i16);
            if pchar == t[col] {
                s1 = s.h[row - width + j - 1] + SCORE_MATCH;
                let mut b = s.b[col];
                consecutive = s.c[row - width + j - 1] + 1;
                if consecutive > 1 {
                    let fb = s.b[col + 1 - consecutive as usize];
                    if b >= BONUS_BOUNDARY && b > fb {
                        consecutive = 1;
                    } else {
                        b = max(b, max(BONUS_CONSECUTIVE, fb));
                    }
                }
                if s1 + b < s2 {
                    s1 += s.b[col];
                    consecutive = 0;
                } else {
                    s1 += b;
                }
            }
            s.c[row + j] = consecutive;
            in_gap = s1 < s2;
            let score = max(max(s1, s2), 0);
            if pidx == m - 1 && score > max_score {
                (max_score, max_pos) = (score, col);
            }
            s.h[row + j] = score;
        }
    }

    // Phase 4: the positions, back from the best cell.
    let mut j = f0;
    let positions = with_pos.then(|| {
        let (mut i, mut prefer_match, mut pos) = (m - 1, true, Vec::with_capacity(m));
        j = max_pos;
        loop {
            let (ii, j0) = (i * width, j - f0);
            let score = s.h[ii + j0];
            let s1 = if i > 0 && j >= s.f[i] { s.h[ii - width + j0 - 1] } else { 0 };
            let s2 = if j > s.f[i] { s.h[ii + j0 - 1] } else { 0 };
            let row = i;
            if score > s1 && (score > s2 || score == s2 && prefer_match) {
                pos.push(j + min);
                if i == 0 {
                    break;
                }
                i -= 1;
            }
            prefer_match = s.c[ii + j0] > 1
                || row + 1 < m && j < last && j + 1 >= s.f[row + 1] && s.c[ii + width + j0 + 1] > 0;
            j -= 1;
        }
        pos
    });
    Some(Found { start: min + j, end: min + max_pos + 1, score: max_score as i32, positions })
}

/// calculateScore: the score of PATTERN matched in order within START..END.
fn calculate_score(text: &[u32], classes: &[Class], pattern: &[u32], start: usize, end: usize, with_pos: bool)
    -> (i32, Option<Vec<usize>>) {
    let (mut pidx, mut score, mut in_gap, mut consecutive, mut first_bonus) = (0, 0i32, false, 0, 0i16);
    let mut pos = with_pos.then(|| Vec::with_capacity(pattern.len()));
    let mut prev_class = if start > 0 { classes[start - 1] } else { INITIAL_CLASS };
    for idx in start..end {
        let class = classes[idx];
        if pidx < pattern.len() && text[idx] == pattern[pidx] {
            if let Some(pos) = pos.as_mut() {
                pos.push(idx);
            }
            score += SCORE_MATCH as i32;
            let mut b = bonus(prev_class, class);
            if consecutive == 0 {
                first_bonus = b;
            } else {
                if b >= BONUS_BOUNDARY && b > first_bonus {
                    first_bonus = b;
                }
                b = max(b, max(first_bonus, BONUS_CONSECUTIVE));
            }
            score += if pidx == 0 { b * BONUS_FIRST_CHAR_MULTIPLIER } else { b } as i32;
            in_gap = false;
            consecutive += 1;
            pidx += 1;
        } else {
            score += if in_gap { SCORE_GAP_EXTENSION } else { SCORE_GAP_START } as i32;
            in_gap = true;
            consecutive = 0;
            first_bonus = 0;
        }
        prev_class = class;
    }
    (score, pos)
}

/// FuzzyMatchV1: the first occurrence, shortened from its end.
pub(crate) fn fuzzy_v1(text: &[u32], classes: &[Class], pattern: &[u32], with_pos: bool) -> Option<Found> {
    if pattern.is_empty() {
        return found(0, 0, 0);
    }
    fuzzy_index(text, pattern)?;
    let (mut pidx, mut start, mut end) = (0, None, None);
    for (index, &ch) in text.iter().enumerate() {
        if ch == pattern[pidx] {
            start.get_or_insert(index);
            pidx += 1;
            if pidx == pattern.len() {
                end = Some(index + 1);
                break;
            }
        }
    }
    let (mut start, end) = (start?, end?);
    pidx -= 1;
    for index in (start..end).rev() {
        if text[index] == pattern[pidx] {
            if pidx == 0 {
                start = index;
                break;
            }
            pidx -= 1;
        }
    }
    let (score, positions) = calculate_score(text, classes, pattern, start, end, with_pos);
    Some(Found { start, end, score, positions })
}

/// ExactMatchNaive, and ExactMatchBoundary when BOUNDARY: the occurrence
/// whose first character has the highest bonus.
pub(crate) fn exact(text: &[u32], classes: &[Class], pattern: &[u32], boundary: bool) -> Option<Found> {
    let (n, m) = (text.len(), pattern.len());
    if m == 0 {
        return found(0, 0, 0);
    }
    if n < m {
        return None;
    }
    fuzzy_index(text, pattern)?;
    let (mut pidx, mut best, mut b, mut bb, mut best_bonus) = (0, None, 0i16, 0i16, -1i16);
    let mut index = 0;
    while index < n {
        let mut ok = pattern[pidx] == text[index];
        if ok {
            if pidx == 0 {
                b = bonus_at(classes, index);
            }
            if boundary {
                if pidx == 0 {
                    bb = b;
                }
                ok = bb >= BONUS_BOUNDARY;
                if ok && pidx == 0 {
                    ok = index == 0 || classes[index - 1] <= Class::Delimiter;
                }
                if ok && pidx == m - 1 {
                    ok = index == n - 1 || classes[index + 1] <= Class::Delimiter;
                }
            }
        }
        if ok {
            pidx += 1;
            if pidx == m {
                if b > best_bonus {
                    (best, best_bonus) = (Some(index), b);
                }
                if b >= BONUS_BOUNDARY {
                    break;
                }
                index -= pidx - 1;
                (pidx, b) = (0, 0);
            }
        } else {
            index -= pidx;
            (pidx, b) = (0, 0);
        }
        index += 1;
    }
    let end = best? + 1;
    let start = end - m;
    let score = if boundary {
        // fzf scores with the bonus of the scan's last state.
        let underscore = '_' as u32;
        let (mut score, mut deduct) = (b as i32, (b - BONUS_BOUNDARY) as i32 + 1);
        if start > 0 && text[start - 1] == underscore {
            score -= deduct + 1;
            deduct = 1;
        }
        if end < n && text[end] == underscore {
            score -= deduct;
        }
        score + SCORE_MATCH as i32 * m as i32 + BONUS_BOUNDARY_WHITE as i32 * (m as i32 + 1)
    } else {
        calculate_score(text, classes, pattern, start, end, false).0
    };
    found(start, end, score)
}

fn leading_spaces(text: &[u32]) -> usize {
    text.iter().take_while(|&&c| is_space(c)).count()
}

fn trailing_spaces(text: &[u32]) -> usize {
    text.iter().rev().take_while(|&&c| is_space(c)).count()
}

/// PrefixMatch, after the text's leading white space unless PATTERN
/// starts with some.
pub(crate) fn prefix(text: &[u32], classes: &[Class], pattern: &[u32]) -> Option<Found> {
    let Some(&first) = pattern.first() else { return found(0, 0, 0) };
    let start = if is_space(first) { 0 } else { leading_spaces(text) };
    let end = start + pattern.len();
    if end > text.len() || text[start..end] != *pattern {
        return None;
    }
    found(start, end, calculate_score(text, classes, pattern, start, end, false).0)
}

/// SuffixMatch, before the text's trailing white space unless PATTERN
/// ends with some.
pub(crate) fn suffix(text: &[u32], classes: &[Class], pattern: &[u32]) -> Option<Found> {
    let trim = pattern.last().is_none_or(|&c| !is_space(c));
    let end = text.len() - if trim { trailing_spaces(text) } else { 0 };
    if pattern.is_empty() {
        return found(end, end, 0);
    }
    let start = end.checked_sub(pattern.len())?;
    if text[start..end] != *pattern {
        return None;
    }
    found(start, end, calculate_score(text, classes, pattern, start, end, false).0)
}

/// EqualMatch: the whole text but its surrounding white space.
pub(crate) fn equal(text: &[u32], pattern: &[u32]) -> Option<Found> {
    let (&first, &last) = (pattern.first()?, pattern.last()?);
    let lead = if is_space(first) { 0 } else { leading_spaces(text) };
    let trail = if is_space(last) { 0 } else { trailing_spaces(text) };
    if text.len() as isize - lead as isize - trail as isize != pattern.len() as isize
        || text[lead..lead + pattern.len()] != *pattern {
        return None;
    }
    let m = pattern.len() as i32;
    let white = BONUS_BOUNDARY_WHITE as i32;
    found(lead, lead + pattern.len(), (SCORE_MATCH as i32 + white) * m + (BONUS_FIRST_CHAR_MULTIPLIER as i32 - 1) * white)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Fuzzy,
    Exact,
    ExactBoundary,
    Prefix,
    Suffix,
    Equal,
}

/// One term of the extended syntax.  TEXT is folded by the caller when
/// the term ignores case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Term {
    pub kind: Kind,
    pub inverse: bool,
    pub text: Vec<u32>,
    pub sensitive: bool,
}

/// A query: every set must match, a set when any of its terms does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub sets: Vec<Vec<Term>>,
    /// Whether matches rank by score: fzf keeps the input order when
    /// every term is inverse.
    pub sortable: bool,
}

const SPACE: u32 = ' ' as u32;
const BACKSLASH: u32 = '\\' as u32;

/// The tokens fzf splits QUERY into: on runs of spaces, `\ ' standing for
/// a space inside a token.  (fzf marks `\ ' with a tab, so a typed tab
/// becomes a space there; here it stays a tab.)
fn tokens(query: &[u32]) -> Vec<Vec<u32>> {
    let (mut out, mut token, mut i) = (vec![], vec![], 0);
    while i < query.len() {
        match query[i] {
            BACKSLASH if query.get(i + 1) == Some(&SPACE) => {
                token.push(SPACE);
                i += 1;
            }
            SPACE => {
                if !token.is_empty() {
                    out.push(std::mem::take(&mut token));
                }
            }
            c => token.push(c),
        }
        i += 1;
    }
    if !token.is_empty() {
        out.push(token);
    }
    out
}

/// parseTerms in fzf's default fuzzy mode.  SENSITIVE tells, from a
/// token's text, whether its term respects case.
pub(crate) fn parse(query: &[u32], sensitive: &dyn Fn(&[u32]) -> bool) -> Query {
    let is = |t: &[u32], s: &str| t.iter().copied().eq(s.chars().map(|c| c as u32));
    let (mut sets, mut set): (Vec<Vec<Term>>, Vec<Term>) = (vec![], vec![]);
    let (mut switch_set, mut after_bar) = (false, false);
    for token in tokens(query) {
        let case = sensitive(&token);
        if !set.is_empty() && !after_bar && is(&token, "|") {
            (switch_set, after_bar) = (false, true);
            continue;
        }
        after_bar = false;
        let (mut kind, mut inverse, mut text) = (Kind::Fuzzy, false, &token[..]);
        if text.first() == Some(&('!' as u32)) {
            (inverse, kind, text) = (true, Kind::Exact, &text[1..]);
        }
        if !is(text, "$") && text.last() == Some(&('$' as u32)) {
            (kind, text) = (Kind::Suffix, &text[..text.len() - 1]);
        }
        let quote = '\'' as u32;
        if text.len() > 2 && text[0] == quote && text[text.len() - 1] == quote {
            (kind, text) = (Kind::ExactBoundary, &text[1..text.len() - 1]);
        } else if text.first() == Some(&quote) {
            kind = if inverse { Kind::Fuzzy } else { Kind::Exact };
            text = &text[1..];
        } else if text.first() == Some(&('^' as u32)) {
            kind = if kind == Kind::Suffix { Kind::Equal } else { Kind::Prefix };
            text = &text[1..];
        }
        if !text.is_empty() {
            if switch_set {
                sets.push(std::mem::take(&mut set));
            }
            set.push(Term { kind, inverse, text: text.to_vec(), sensitive: case });
            switch_set = true;
        }
    }
    if !set.is_empty() {
        sets.push(set);
    }
    let sortable = sets.iter().flatten().any(|t| !t.inverse);
    Query { sets, sortable }
}

/// A candidate's codes as each term matches them.
#[derive(Clone, Copy)]
pub(crate) struct Item<'a> {
    pub original: &'a [u32],
    pub folded: &'a [u32],
}

fn run(s: &mut Scratch, term: &Term, item: Item<'_>, classes: &[Class], with_pos: bool) -> Option<Found> {
    let text = if term.sensitive { item.original } else { item.folded };
    let p = &term.text[..];
    match term.kind {
        Kind::Fuzzy => fuzzy_v2(s, text, classes, p, with_pos),
        Kind::Exact => exact(text, classes, p, false),
        Kind::ExactBoundary => exact(text, classes, p, true),
        Kind::Prefix => prefix(text, classes, p),
        Kind::Suffix => suffix(text, classes, p),
        Kind::Equal => equal(text, p),
    }
}

/// extendedMatch: ITEM's total score under QUERY, and when WITH_POS the
/// positions to highlight; None when a set does not match.
pub(crate) fn match_item(s: &mut Scratch, query: &Query, item: Item<'_>, with_pos: bool) -> Option<(i32, Vec<usize>)> {
    let mut classes = std::mem::take(&mut s.classes);
    classes.clear();
    classes.extend(item.original.iter().map(|&c| class_of(c)));
    let found = match_sets(s, query, item, &classes, with_pos);
    s.classes = classes;
    found
}

fn match_sets(s: &mut Scratch, query: &Query, item: Item<'_>, classes: &[Class], with_pos: bool) -> Option<(i32, Vec<usize>)> {
    let (mut total, mut positions) = (0, vec![]);
    for set in &query.sets {
        let mut score = None;
        for term in set {
            match run(s, term, item, classes, with_pos) {
                Some(_) if term.inverse => continue,
                Some(hit) => {
                    score = Some(hit.score);
                    if with_pos {
                        positions.extend(hit.positions.unwrap_or_else(|| (hit.start..hit.end).collect()));
                    }
                    break;
                }
                None if term.inverse => score = Some(0),
                None => {}
            }
        }
        total += score?;
    }
    Some((total, positions))
}

/// Candidates decoded for one pass: plain buffers that no Lisp value
/// reaches, so scoring may run on other threads.  Character classes are
/// computed there, per candidate.
#[derive(Default)]
pub(crate) struct Corpus {
    pub original: Vec<u32>,
    /// Folded codes, parallel to ORIGINAL; empty when no term folds.
    pub folded: Vec<u32>,
    pub bounds: Vec<(usize, usize)>,
}

impl Corpus {
    /// Start a candidate; push its codes, then call `end`.
    pub(crate) fn end(&mut self, start: usize) {
        self.bounds.push((start, self.original.len()));
    }

    pub(crate) fn item(&self, i: usize) -> Item<'_> {
        let (a, b) = self.bounds[i];
        let folded = if self.folded.is_empty() { &self.original[a..b] } else { &self.folded[a..b] };
        Item { original: &self.original[a..b], folded }
    }
}

/// fzf's rank of a match: higher score, then shorter trimmed text, then
/// the input order.  Scores and lengths saturate at 16 bits as fzf's do.
fn rank_key(score: i32, item: Item<'_>, index: usize) -> (Reverse<u16>, u16, usize) {
    let text = item.original;
    let lead = leading_spaces(text);
    let len = if lead == text.len() { 0 } else { text.len() - lead - trailing_spaces(text) };
    (Reverse(score.clamp(0, u16::MAX as i32) as u16), len.min(u16::MAX as usize) as u16, index)
}

/// How often a pass looks at its generation, in candidates.
const CANCEL_STRIDE: usize = 256;

fn score_range(query: &Query, corpus: &Corpus, range: std::ops::Range<usize>, stale: &(dyn Fn() -> bool + Sync))
    -> Option<Vec<(usize, i32)>> {
    let mut s = Scratch::default();
    let mut out = vec![];
    for i in range {
        if i % CANCEL_STRIDE == 0 && stale() {
            return None;
        }
        if let Some((score, _)) = match_item(&mut s, query, corpus.item(i), false) {
            out.push((i, score));
        }
    }
    Some(out)
}

/// The indexes of CORPUS's candidates QUERY matches, in fzf's rank order
/// (input order when QUERY is not sortable).  At PARALLEL candidates or
/// more the scoring is split across threads; the order is the same.
/// None when STALE says a newer pass superseded this one.
pub(crate) fn rank(query: &Query, corpus: &Corpus, parallel: Option<usize>, stale: &(dyn Fn() -> bool + Sync))
    -> Option<Vec<usize>> {
    let n = corpus.bounds.len();
    let mut scored = if parallel.is_some_and(|at| n >= at.max(1)) {
        use rayon::prelude::*;
        let chunk = n.div_ceil(rayon::current_num_threads() * 4).max(CANCEL_STRIDE);
        let starts: Vec<usize> = (0..n).step_by(chunk).collect();
        let parts: Vec<Option<Vec<(usize, i32)>>> = starts.par_iter()
            .map(|&a| score_range(query, corpus, a..(a + chunk).min(n), stale))
            .collect();
        parts.into_iter().collect::<Option<Vec<_>>>()?.concat()
    } else {
        score_range(query, corpus, 0..n, stale)?
    };
    if query.sortable {
        scored.sort_unstable_by_key(|&(i, score)| rank_key(score, corpus.item(i), i));
    }
    Some(scored.into_iter().map(|(i, _)| i).collect())
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Make every completion pass in progress stale, so that it stops early:
/// for a host that has newer input than the pass was started for.
pub fn advance_completion_generation() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The completion generation a pass starts in.
pub(crate) struct Generation(u64);

impl Generation {
    pub(crate) fn current() -> Self {
        Generation(GENERATION.load(Ordering::Relaxed))
    }

    pub(crate) fn stale(&self) -> bool {
        GENERATION.load(Ordering::Relaxed) != self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(s: &str) -> Vec<u32> {
        s.chars().map(|c| c as u32).collect()
    }

    type Algo = fn(&[u32], &[Class], &[u32]) -> Option<Found>;

    fn v2(t: &[u32], c: &[Class], p: &[u32]) -> Option<Found> {
        fuzzy_v2(&mut Scratch::default(), t, c, p, true)
    }
    fn v1(t: &[u32], c: &[Class], p: &[u32]) -> Option<Found> {
        fuzzy_v1(t, c, p, true)
    }
    fn naive(t: &[u32], c: &[Class], p: &[u32]) -> Option<Found> {
        exact(t, c, p, false)
    }
    fn equal_match(t: &[u32], _: &[Class], p: &[u32]) -> Option<Found> {
        equal(t, p)
    }

    /// algo_test.go's assertMatch, forward: the range is the positions'
    /// when the algorithm gives some, and (-1, -1, 0) is no match.
    #[track_caller]
    fn assert_match(algo: Algo, case_sensitive: bool, input: &str, pattern: &str, sidx: i64, eidx: i64, score: i32) {
        let pattern = if case_sensitive { pattern.to_string() } else { pattern.to_lowercase() };
        let original = codes(input);
        let classes: Vec<Class> = original.iter().map(|&c| class_of(c)).collect();
        let text = if case_sensitive { original.clone() } else { codes(&input.to_lowercase()) };
        let got = algo(&text, &classes, &codes(&pattern)).map(|f| match f.positions {
            Some(mut p) if !p.is_empty() => {
                p.sort();
                (p[0] as i64, p[p.len() - 1] as i64 + 1, f.score)
            }
            _ => (f.start as i64, f.end as i64, f.score),
        });
        assert_eq!(got.unwrap_or((-1, -1, 0)), (sidx, eidx, score), "{input:?} / {pattern:?}");
    }

    const M: i32 = SCORE_MATCH as i32;
    const GS: i32 = SCORE_GAP_START as i32;
    const GE: i32 = SCORE_GAP_EXTENSION as i32;
    const B: i32 = BONUS_BOUNDARY as i32;
    const BN: i32 = BONUS_NON_WORD as i32;
    const BC: i32 = BONUS_CAMEL123 as i32;
    const CONSEC: i32 = BONUS_CONSECUTIVE as i32;
    const MULT: i32 = BONUS_FIRST_CHAR_MULTIPLIER as i32;
    const BW: i32 = BONUS_BOUNDARY_WHITE as i32;
    const BD: i32 = BONUS_BOUNDARY_DELIMITER as i32;

    #[test]
    fn fuzzy_match() {
        for algo in [v1 as Algo, v2] {
            assert_match(algo, false, "fooBarbaz1", "oBZ", 2, 9, M * 3 + BC + GS + GE * 3);
            assert_match(algo, false, "foo bar baz", "fbb", 0, 9, M * 3 + BW * MULT + BW * 2 + 2 * GS + 4 * GE);
            assert_match(algo, false, "/AutomatorDocument.icns", "rdoc", 9, 13, M * 4 + BC + CONSEC * 2);
            assert_match(algo, false, "/man1/zshcompctl.1", "zshc", 6, 10, M * 4 + BD * MULT + BD * 3);
            assert_match(algo, false, "/.oh-my-zsh/cache", "zshc", 8, 13, M * 4 + B * MULT + B * 2 + GS + BD);
            assert_match(algo, false, ".vimrc", ".vimrc", 0, 6, M * 6 + BW * (MULT + 5));
            assert_match(algo, false, "/.vimrc", ".vimrc", 1, 7, M * 6 + BD * (MULT + 5));
            assert_match(algo, false, "a.vimrc", ".vimrc", 1, 7, M * 6 + B * (MULT + 5));
            assert_match(algo, false, "ab0123 456", "12356", 3, 10, M * 5 + CONSEC * 3 + GS + GE);
            assert_match(algo, false, "abc123 456", "12356", 3, 10, M * 5 + BC * MULT + BC * 2 + CONSEC + GS + GE);
            assert_match(algo, false, "foo/bar/baz", "fbb", 0, 9, M * 3 + BW * MULT + BD * 2 + 2 * GS + 4 * GE);
            assert_match(algo, false, "fooBarBaz", "fbb", 0, 7, M * 3 + BW * MULT + BC * 2 + 2 * GS + 2 * GE);
            assert_match(algo, false, "foo barbaz", "fbb", 0, 8, M * 3 + BW * MULT + BW + GS * 2 + GE * 3);
            assert_match(algo, false, "fooBar Baz", "foob", 0, 4, M * 4 + BW * MULT + BW * 3);
            assert_match(algo, false, "xFoo-Bar Baz", "foo-b", 1, 6, M * 5 + BC * MULT + BC * 2 + BN + B);

            assert_match(algo, true, "fooBarbaz", "oBz", 2, 9, M * 3 + BC + GS + GE * 3);
            assert_match(algo, true, "Foo/Bar/Baz", "FBB", 0, 9, M * 3 + BW * MULT + BD * 2 + GS * 2 + GE * 4);
            assert_match(algo, true, "FooBarBaz", "FBB", 0, 7, M * 3 + BW * MULT + BC * 2 + GS * 2 + GE * 2);
            assert_match(algo, true, "FooBar Baz", "FooB", 0, 4, M * 4 + BW * MULT + BW * 2 + max(BC, BW));

            // Consecutive bonus updated
            assert_match(algo, true, "foo-bar", "o-ba", 2, 6, M * 4 + B * 3);

            // Non-match
            assert_match(algo, true, "fooBarbaz", "oBZ", -1, -1, 0);
            assert_match(algo, true, "Foo Bar Baz", "fbb", -1, -1, 0);
            assert_match(algo, true, "fooBarbaz", "fooBarbazz", -1, -1, 0);
        }
    }

    #[test]
    fn exact_match_naive() {
        assert_match(naive, true, "fooBarbaz", "oBA", -1, -1, 0);
        assert_match(naive, true, "fooBarbaz", "fooBarbazz", -1, -1, 0);
        assert_match(naive, false, "fooBarbaz", "oBA", 2, 5, M * 3 + BC + CONSEC);
        assert_match(naive, false, "/AutomatorDocument.icns", "rdoc", 9, 13, M * 4 + BC + CONSEC * 2);
        assert_match(naive, false, "/man1/zshcompctl.1", "zshc", 6, 10, M * 4 + BD * (MULT + 3));
        assert_match(naive, false, "/.oh-my-zsh/cache", "zsh/c", 8, 13, M * 5 + B * (MULT + 3) + BD);
        // TestExactMatchNaiveBackward, forward half.
        assert_match(naive, false, "foobar foob", "oo", 1, 3, M * 2 + CONSEC);
    }

    #[test]
    fn prefix_match() {
        let score = M * 3 + BW * MULT + BW * 2;
        let pre = |t: &[u32], c: &[Class], p: &[u32]| prefix(t, c, p);
        assert_match(pre, true, "fooBarbaz", "Foo", -1, -1, 0);
        assert_match(pre, false, "fooBarBaz", "baz", -1, -1, 0);
        assert_match(pre, false, "fooBarbaz", "Foo", 0, 3, score);
        assert_match(pre, false, "foOBarBaZ", "foo", 0, 3, score);
        assert_match(pre, false, "f-oBarbaz", "f-o", 0, 3, score);
        assert_match(pre, false, " fooBar", "foo", 1, 4, score);
        assert_match(pre, false, " fooBar", " fo", 0, 3, score);
        assert_match(pre, false, "     fo", "foo", -1, -1, 0);
    }

    #[test]
    fn suffix_match() {
        let suf = |t: &[u32], c: &[Class], p: &[u32]| suffix(t, c, p);
        assert_match(suf, true, "fooBarbaz", "Baz", -1, -1, 0);
        assert_match(suf, false, "fooBarbaz", "Foo", -1, -1, 0);
        assert_match(suf, false, "fooBarbaz", "baz", 6, 9, M * 3 + CONSEC * 2);
        assert_match(suf, false, "fooBarBaZ", "baz", 6, 9, (M + BC) * 3 + BC * (MULT - 1));
        assert_match(suf, false, "fooBarbaz ", "baz", 6, 9, M * 3 + CONSEC * 2);
        assert_match(suf, false, "fooBarbaz ", "baz ", 6, 10, M * 4 + CONSEC * 2 + BW);
    }

    #[test]
    fn empty_pattern() {
        let pre = |t: &[u32], c: &[Class], p: &[u32]| prefix(t, c, p);
        let suf = |t: &[u32], c: &[Class], p: &[u32]| suffix(t, c, p);
        assert_match(v1, true, "foobar", "", 0, 0, 0);
        assert_match(v2, true, "foobar", "", 0, 0, 0);
        assert_match(naive, true, "foobar", "", 0, 0, 0);
        assert_match(pre, true, "foobar", "", 0, 0, 0);
        assert_match(suf, true, "foobar", "", 6, 6, 0);
        assert_match(equal_match, true, "foobar", "", -1, -1, 0);
    }

    #[test]
    fn long_string() {
        // TestLongString: past fzf's slab, V2 takes V1, with the same answer.
        let mut input = "x".repeat(u16::MAX as usize * 2);
        input.replace_range(u16::MAX as usize..u16::MAX as usize + 1, "z");
        let at = u16::MAX as i64;
        assert_match(v2, true, &input, "zx", at, at + 2, M * 2 + CONSEC);
    }

    #[test]
    fn positions_are_the_best_alignment() {
        // TestResultPositionsWithReusedSlab: a dirty scratch changes nothing.
        let (target, dirty) = (codes("core_color/view/server.txt"), codes("completion/keybinding/client/handler/writer_index.txt"));
        let classes = |t: &[u32]| t.iter().map(|&c| class_of(c)).collect::<Vec<_>>();
        let fresh = fuzzy_v2(&mut Scratch::default(), &target, &classes(&target), &codes("co/"), true);
        let mut s = Scratch::default();
        fuzzy_v2(&mut s, &dirty, &classes(&dirty), &codes("co/"), true);
        assert_eq!(fuzzy_v2(&mut s, &target, &classes(&target), &codes("co/"), true), fresh);
    }

    fn query(q: &str) -> Query {
        parse(&codes(q), &|t| t.iter().any(|&c| char::from_u32(c).is_some_and(char::is_uppercase)))
    }

    fn term(kind: Kind, inverse: bool, text: &str) -> Term {
        Term { kind, inverse, text: codes(text), sensitive: text.chars().any(char::is_uppercase) }
    }

    #[test]
    fn parses_the_extended_syntax() {
        // pattern_test.go TestParseTermsExtended, fuzzy mode.
        let q = query("aaa 'bbb ^ccc ddd$ !eee !'fff !^ggg !hhh$ | ^iii$ ^xxx | 'yyy | zzz$ | !ZZZ |");
        use Kind::*;
        assert_eq!(q.sets, vec![
            vec![term(Fuzzy, false, "aaa")],
            vec![term(Exact, false, "bbb")],
            vec![term(Prefix, false, "ccc")],
            vec![term(Suffix, false, "ddd")],
            vec![term(Exact, true, "eee")],
            vec![term(Fuzzy, true, "fff")],
            vec![term(Prefix, true, "ggg")],
            vec![term(Suffix, true, "hhh"), term(Equal, false, "iii")],
            vec![term(Prefix, false, "xxx"), term(Exact, false, "yyy"), term(Suffix, false, "zzz"), term(Exact, true, "ZZZ")],
        ]);
        assert!(q.sortable);
        assert_eq!(query("'abc'").sets, vec![vec![term(ExactBoundary, false, "abc")]]);
        assert_eq!(query("a\\ b  ").sets, vec![vec![term(Fuzzy, false, "a b")]]);
        assert_eq!(query("| a").sets, vec![vec![term(Fuzzy, false, "|")], vec![term(Fuzzy, false, "a")]]);
        assert!(query("").sets.is_empty() && !query("").sortable);
        assert!(!query("!a !b").sortable);
        // TestParseTermsEmpty.
        assert!(query("' ^ !' !^").sets.is_empty());
    }

    #[test]
    fn exact_and_equal_terms() {
        // TestExact and TestEqual.
        let text = codes("aabbcc abc");
        let classes: Vec<Class> = text.iter().map(|&c| class_of(c)).collect();
        let q = query("'abc");
        let hit = exact(&text, &classes, &q.sets[0][0].text, false).unwrap();
        assert_eq!((hit.start, hit.end, hit.positions), (7, 10, None));
        let q = query("^AbC$");
        assert_eq!(q.sets[0][0].kind, Kind::Equal);
        for (s, range) in [("ABC", None), ("AbC", Some((0, 3))), ("AbC  ", Some((0, 3))), (" AbC ", Some((1, 4))),
                           ("  AbC", Some((2, 5)))] {
            assert_eq!(equal(&codes(s), &q.sets[0][0].text).map(|f| (f.start, f.end)), range, "{s:?}");
        }
    }

    fn corpus(items: &[&str], fold: bool) -> Corpus {
        let mut c = Corpus::default();
        for item in items {
            let start = c.original.len();
            c.original.extend(codes(item));
            if fold {
                c.folded.extend(codes(&item.to_lowercase()));
            }
            c.end(start);
        }
        c
    }

    fn ranked(q: &str, items: &[&str]) -> Vec<String> {
        let c = corpus(items, true);
        rank(&query(q), &c, None, &|| false).unwrap().into_iter().map(|i| items[i].to_string()).collect()
    }

    #[test]
    fn ranks_by_score_then_length_then_input() {
        // "ff": the word boundary beats a run, and a gap of 8 cancels it.
        assert_eq!(ranked("ff", &["fuzzyfinder", "fuzzy-finder"]), ["fuzzy-finder", "fuzzyfinder"]);
        assert_eq!(ranked("foob", &["foo-bar", "foobar"]), ["foobar", "foo-bar"]);
        assert_eq!(ranked("ab", &["xab-long", "xab", "zab", "nomatch"]), ["xab", "zab", "xab-long"]);
        assert_eq!(ranked("!x", &["bb", "a", "xc"]), ["bb", "a"]);
        assert_eq!(ranked("^fi | txt$ !old", &["file.org", "old.txt", "a.txt", "notes"]), ["a.txt", "file.org"]);
        assert_eq!(ranked("'abc'", &["xabc", "x abc y", "abc_d"]), ["x abc y", "abc_d"]);
        assert_eq!(ranked("^abc$", &[" abc ", "abcd"]), [" abc "]);
        // Smart case: an upper-case term respects case.
        assert_eq!(ranked("Fo", &["foo", "Foo"]), ["Foo"]);
    }

    #[test]
    fn highlights_every_term() {
        let c = corpus(&["foo/bar.txt"], true);
        let hit = match_item(&mut Scratch::default(), &query("fb 'txt !zzz"), c.item(0), true).unwrap();
        assert_eq!(hit.1, vec![4, 0, 8, 9, 10]);
    }

    /// The synthetic corpus of fzf's TestFuzzyMatchV2TwoEquivalence.
    fn synthetic(count: usize) -> Vec<String> {
        let words = ["src", "main", "core", "config", "parser", "render", "server", "client", "index", "handler",
                     "util", "list", "cache", "reader"];
        let exts = [".go", ".rb", ".py", ".md", ".c", ".txt"];
        let mut seed = 12345u32;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as usize % n
        };
        (0..count).map(|_| {
            let depth = 2 + next(4);
            let mut s = String::new();
            for d in 0..depth {
                if d > 0 {
                    s.push('/');
                }
                s.push_str(words[next(words.len())]);
                if next(5) == 0 {
                    s.push('_');
                    s.push_str(words[next(words.len())]);
                }
            }
            s + exts[next(exts.len())]
        }).collect()
    }

    #[test]
    fn parallel_equals_serial() {
        let items = synthetic(20_000);
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let c = corpus(&refs, true);
        for q in ["co", "sr", "cfg", "s/c", "'core ^src !txt", "re | ma .go$", "Co", "", "!py"] {
            let q = query(q);
            let serial = rank(&q, &c, None, &|| false).unwrap();
            assert_eq!(rank(&q, &c, Some(1), &|| false).unwrap(), serial);
            assert_eq!(rank(&q, &c, Some(2048), &|| false).unwrap(), serial);
        }
    }

    #[test]
    fn a_stale_pass_stops() {
        let items = synthetic(10_000);
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let c = corpus(&refs, true);
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let stale = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        assert_eq!(rank(&query("co"), &c, None, &stale), None);
        assert!(checks.load(Ordering::Relaxed) <= 4, "stopped at the first stale check");
        checks.store(0, Ordering::Relaxed);
        assert_eq!(rank(&query("co"), &c, Some(1), &stale), None);
        let generation = Generation::current();
        assert!(!generation.stale());
        advance_completion_generation();
        assert!(generation.stale() && !Generation::current().stale());
    }
}
