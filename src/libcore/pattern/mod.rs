// Copyright 2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! The pattern API.

// FIXME: This API has not been RFC'ed yet. It implements the interface sketched
// in RFC 2295 for `OsStr`, but a new RFC should be submitted to apply it for
// `str` and `[T]`.
//
// FIXME: Improve documentation.

#![unstable(feature = "generic_pattern", issue = "0")]

use ops::Range;
use borrow::Borrow;

mod iterators;

pub use self::iterators::{
    Split, RSplit, SplitN, RSplitN, SplitTerminator, RSplitTerminator,
    Matches, RMatches, MatchIndices, RMatchIndices, MatchRanges, RMatchRanges,
    ReplaceWith,
};

/// A generic pattern.
pub trait Pattern<H: Haystack>: Sized {
    /// Associated searcher for this pattern
    type Searcher: Searcher<H>;

    /// Constructs the associated searcher from `self` and the `haystack` to
    /// search in.
    fn into_searcher(self, haystack: H) -> Self::Searcher;

    /// Checks whether the pattern matches anywhere in the haystack
    fn is_contained_in(self, haystack: H) -> bool;

    /// Checks whether the pattern matches at the front of the haystack
    fn is_prefix_of(self, haystack: H) -> bool;

    /// Checks whether the pattern matches at the back of the haystack
    fn is_suffix_of(self, haystack: H) -> bool where Self::Searcher: ReverseSearcher<H>;
}

/// A searcher for a generic pattern.
pub trait Searcher<H: Haystack> {
    /// Getter for the underlying haystack to be searched in.
    ///
    /// Will always return the same instance.
    fn haystack(&self) -> H;

    /// Finds the next range in the haystack which matches the pattern.
    fn next_match(&mut self) -> Option<(H::StartCursor, H::EndCursor)>;

    /// Finds the next range in the haystack which *does not* match the pattern.
    fn next_reject(&mut self) -> Option<(H::StartCursor, H::EndCursor)>;
}

/// A reverse searcher for a generic pattern.
pub trait ReverseSearcher<H: Haystack>: Searcher<H> {
    /// Finds the next range from the back in the haystack which matches the
    /// pattern.
    fn next_match_back(&mut self) -> Option<(H::StartCursor, H::EndCursor)>;

    /// Finds the next range from the back in the haystack which *does not*
    /// match the pattern.
    fn next_reject_back(&mut self) -> Option<(H::StartCursor, H::EndCursor)>;
}

/// A marker trait to express that a `ReverseSearcher` can be used for a
/// `DoubleEndedIterator` implementation.
///
/// For this, the impl of `Searcher` and `ReverseSearcher` need to follow these
/// conditions:
///
/// - All results of `next_*()` need to be identical to the results of
///     `next_*_back()` in reverse order.
/// - `next_*()` and `next_*_back()` need to behave as the two ends of a range
///     of values, that is they can not "walk past each other".
pub trait DoubleEndedSearcher<H: Haystack>: ReverseSearcher<H> {}

/// An extension trait providing methods for replacing
pub trait ReplaceOutput<H>: Borrow<H> {
    /// Creates an owned empty replacement result.
    fn new_replace_output() -> Self;

    /// Extends a haystack to the end of the replacement result.
    fn extend_from_haystack(&mut self, haystack: &H);
}


/// A haystack is a sequence which we can search patterns from it.
pub trait Haystack: Sized {
    /// The type of start cursor of a range of this haystack.
    type StartCursor: Copy + Ord;

    /// The type of end cursor of a range of this haystack.
    type EndCursor: Copy + Ord;

    /// Obtains the cursor at the beginning of this haystack.
    fn cursor_at_front(&self) -> Self::StartCursor;

    /// Obtains the cursor at the end of this haystack.
    fn cursor_at_back(&self) -> Self::EndCursor;

    /// Converts a start cursor to an integer offset.
    ///
    /// # Safety
    ///
    /// The cursor must point to a valid element boundary of the haystack.
    unsafe fn start_cursor_to_offset(&self, cur: Self::StartCursor) -> usize;

    /// Converts an end cursor to an integer offset.
    ///
    /// # Safety
    ///
    /// The cursor must point to a valid element boundary of the haystack.
    unsafe fn end_cursor_to_offset(&self, cur: Self::EndCursor) -> usize;

    /// Obtains a "slice" of the haystack bounded by the two cursors.
    ///
    /// # Safety
    ///
    /// The cursors must point to valid element boundaries of the haystack.
    unsafe fn range_to_self(self, start: Self::StartCursor, end: Self::EndCursor) -> Self;

    /// Converts a start cursor to an end cursor.
    unsafe fn start_to_end_cursor(&self, cur: Self::StartCursor) -> Self::EndCursor;

    /// Converts an end cursor to a start cursor.
    unsafe fn end_to_start_cursor(&self, cur: Self::EndCursor) -> Self::StartCursor;

    //--------------------------------------------------------------------------
    // Extension methods for searching a pattern from a haystack.
    //--------------------------------------------------------------------------

    /// Returns `true` if the given pattern matches a sub-slice of this
    /// haystack.
    ///
    /// Returns `false` if it does not.
    #[inline]
    fn contains<P: Pattern<Self>>(self, pat: P) -> bool {
        pat.is_contained_in(self)
    }

    /// Returns `true` if the given pattern matches a prefix of this haystack.
    ///
    /// Returns `false` if it does not.
    #[inline]
    fn starts_with<P: Pattern<Self>>(self, pat: P) -> bool {
        pat.is_prefix_of(self)
    }

    /// Returns `true` if the given pattern matches a suffix of this haystack.
    ///
    /// Returns `false` if it does not.
    #[inline]
    fn ends_with<P: Pattern<Self>>(self, pat: P) -> bool
    where
        P::Searcher: ReverseSearcher<Self>
    {
        pat.is_suffix_of(self)
    }

    /// Returns the start index of the first sub-slice of this haystack that
    /// matches the pattern.
    ///
    /// Returns [`None`] if the pattern doesn't match.
    fn find<P: Pattern<Self>>(self, pat: P) -> Option<usize> {
        let mut searcher = pat.into_searcher(self);
        let cursor = searcher.next_match()?.0;
        unsafe {
            Some(Haystack::start_cursor_to_offset(&searcher.haystack(), cursor))
        }
    }

    /// Returns the start index of the last sub-slice of this haystack that
    /// matches the pattern.
    ///
    /// Returns [`None`] if the pattern doesn't match.
    fn rfind<P: Pattern<Self>>(self, pat: P) -> Option<usize>
    where
        P::Searcher: ReverseSearcher<Self>
    {
        let mut searcher = pat.into_searcher(self);
        let cursor = searcher.next_match_back()?.0;
        unsafe {
            Some(searcher.haystack().start_cursor_to_offset(cursor))
        }
    }

    /// Returns the range of the first sub-slice of this haystack that matches
    /// the pattern.
    ///
    /// Returns [`None`] if the pattern doesn't match.
    fn find_range<P: Pattern<Self>>(self, pat: P) -> Option<Range<usize>> {
        let mut searcher = pat.into_searcher(self);
        let range = searcher.next_match()?;
        let hs = searcher.haystack();
        unsafe {
            Some(hs.start_cursor_to_offset(range.0) .. hs.end_cursor_to_offset(range.1))
        }
    }

    /// Returns the range of the last sub-slice of this haystack that matches
    /// the pattern.
    ///
    /// Returns [`None`] if the pattern doesn't match.
    fn rfind_range<P: Pattern<Self>>(self, pat: P) -> Option<Range<usize>>
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        let mut searcher = pat.into_searcher(self);
        let range = searcher.next_match_back()?;
        let hs = searcher.haystack();
        unsafe {
            Some(hs.start_cursor_to_offset(range.0) .. hs.end_cursor_to_offset(range.1))
        }
    }

    /// An iterator over sub-slices of this haystack, separated by a pattern.
    #[inline]
    fn split<P: Pattern<Self>>(self, pat: P) -> Split<Self, P> {
        Split(iterators::SplitInternal {
            start: self.cursor_at_front(),
            end: self.cursor_at_back(),
            matcher: pat.into_searcher(self),
            allow_trailing_empty: true,
            finished: false,
        })
    }

    /// An iterator over sub-slices of the given haystack, separated by a
    /// pattern and yielded in reverse order.
    #[inline]
    fn rsplit<P: Pattern<Self>>(self, pat: P) -> RSplit<Self, P>
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        RSplit(self.split(pat).0)
    }

    ///
    #[inline]
    fn splitn<P: Pattern<Self>>(self, count: usize, pat: P) -> SplitN<Self, P> {
        SplitN(iterators::SplitNInternal {
            iter: self.split(pat).0,
            count,
        })
    }

    ///
    #[inline]
    fn rsplitn<P: Pattern<Self>>(self, count: usize, pat: P) -> RSplitN<Self, P>
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        RSplitN(self.splitn(count, pat).0)
    }

    /// An iterator over sub-slices of the given haystack, separated by pattern.
    #[inline]
    fn split_terminator<P: Pattern<Self>>(self, pat: P) -> SplitTerminator<Self, P> {
        SplitTerminator(iterators::SplitInternal {
            allow_trailing_empty: false,
            ..self.split(pat).0
        })
    }

    /// An iterator over sub-slices of `self`, separated by a pattern and
    /// yielded in reverse order.
    #[inline]
    fn rsplit_terminator<P: Pattern<Self>>(self, pat: P) -> RSplitTerminator<Self, P>
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        RSplitTerminator(self.split_terminator(pat).0)
    }

    ///
    #[inline]
    fn matches<P: Pattern<Self>>(self, pat: P) -> Matches<Self, P> {
        Matches(iterators::MatchesInternal(pat.into_searcher(self)))
    }

    ///
    #[inline]
    fn rmatches<P: Pattern<Self>>(self, pat: P) -> RMatches<Self, P>
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        RMatches(self.matches(pat).0)
    }

    ///
    #[inline]
    fn match_indices<P: Pattern<Self>>(self, pat: P) -> MatchIndices<Self, P> {
        MatchIndices(iterators::MatchIndicesInternal(pat.into_searcher(self)))
    }

    ///
    #[inline]
    fn rmatch_indices<P: Pattern<Self>>(self, pat: P) -> RMatchIndices<Self, P>
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        RMatchIndices(self.match_indices(pat).0)
    }

    ///
    #[inline]
    fn match_ranges<P: Pattern<Self>>(self, pat: P) -> MatchRanges<Self, P> {
        MatchRanges(iterators::MatchRangesInternal(pat.into_searcher(self)))
    }

    ///
    #[inline]
    fn rmatch_ranges<P: Pattern<Self>>(self, pat: P) -> RMatchRanges<Self, P>
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        RMatchRanges(self.match_ranges(pat).0)
    }

    ///
    #[inline]
    fn trim_matches<P: Pattern<Self>>(self, pat: P) -> Self
    where
        P::Searcher: DoubleEndedSearcher<Self>
    {
        let mut i = self.cursor_at_front();
        let mut j = unsafe { self.start_to_end_cursor(i) };
        let mut matcher = pat.into_searcher(self);
        if let Some((a, b)) = matcher.next_reject() {
            i = a;
            j = b;
            // Remember earliest known match, correct it below if last match is different
        }
        if let Some((_, b)) = matcher.next_reject_back() {
            j = b;
        }
        unsafe {
            // Searcher is known to return valid indices
            matcher.haystack().range_to_self(i, j)
        }
    }

    ///
    #[inline]
    fn trim_left_matches<P: Pattern<Self>>(self, pat: P) -> Self {
        let end = self.cursor_at_back();
        let mut i = unsafe { self.end_to_start_cursor(end) };
        let mut matcher = pat.into_searcher(self);
        if let Some((a, _)) = matcher.next_reject() {
            i = a;
        }
        unsafe {
            // Searcher is known to return valid indices
            matcher.haystack().range_to_self(i, end)
        }
    }

    ///
    #[inline]
    fn trim_right_matches<P: Pattern<Self>>(self, pat: P) -> Self
    where
        P::Searcher: ReverseSearcher<Self>,
    {
        let start = self.cursor_at_front();
        let mut j = unsafe { self.start_to_end_cursor(start) };
        let mut matcher = pat.into_searcher(self);
        if let Some((_, b)) = matcher.next_reject_back() {
            j = b;
        }
        unsafe {
            // Searcher is known to return valid indices
            matcher.haystack().range_to_self(start, j)
        }
    }

    /// Performs generic replacement.
    #[inline]
    fn replace_with<P, B, F>(self, pat: P, to: F, count: Option<usize>) -> ReplaceWith<Self, P, F>
    where
        P: Pattern<Self>,
        B: From<Self>,
        F: FnMut(Self) -> B,
    {
        ReplaceWith::new(self, pat, to, count)
    }
}
