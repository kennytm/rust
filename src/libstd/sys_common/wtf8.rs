// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Implementation of [the WTF-8](https://simonsapin.github.io/wtf-8/) and
//! [OMG-WTF-8](https://github.com/kennytm/omgwtf8) encodings.
//!
//! This library uses Rust’s type system to maintain
//! [well-formedness](https://simonsapin.github.io/wtf-8/#well-formed),
//! like the `String` and `&str` types do for UTF-8.
//!
//! Since [WTF-8 must not be used
//! for interchange](https://simonsapin.github.io/wtf-8/#intended-audience),
//! this library deliberately does not provide access to the underlying bytes
//! of WTF-8 strings,
//! nor can it decode WTF-8 from arbitrary bytes.
//! WTF-8 strings can be obtained from UTF-8, UTF-16, or code points.

// this module is imported from @SimonSapin's repo and has tons of dead code on
// unix (it's mostly used on windows), so don't worry about dead code here.
#![allow(dead_code)]

use core::str::next_code_point;

use borrow::Cow;
use char;
use fmt;
use hash::{Hash, Hasher};
use mem;
use ops;
use rc::Rc;
use slice;
use str;
use sync::Arc;
use sys_common::AsInner;
use num::NonZeroU16;
use cmp;

const UTF8_REPLACEMENT_CHARACTER: &'static str = "\u{FFFD}";

/// Represents a high surrogate code point.
///
/// Internally, the value is the last 2 bytes of the surrogate in its canonical
/// (WTF-8) representation, e.g. U+D800 is `ed a0 80` in WTF-8, so the value
/// stored here would be `0xa080`. This also means the valid range of this type
/// must be `0xa080..=0xafbf`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct HighSurrogate(NonZeroU16);
impl HighSurrogate {
    fn decode(self) -> [u8; 3] {
        let c = self.0.get();
        [0xed, (c >> 8) as u8, c as u8]
    }
}

/// Represents a low surrogate code point.
///
/// Internally, the value is the last 2 bytes of the surrogate in its canonical
/// (WTF-8) representation, e.g. U+DC00 is `ed b0 80` in WTF-8, so the value
/// stored here would be `0xb080`. This also means the valid range of this type
/// must be `0xb080..=0xbfbf`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct LowSurrogate(NonZeroU16);
impl LowSurrogate {
    #[cfg(test)]
    fn from_code_point_unchecked(cp: u16) -> Self {
        let encoded = cp & 0x3f | (cp << 2) & 0xf00 | 0xb080;
        unsafe { LowSurrogate(NonZeroU16::new_unchecked(encoded)) }
    }

    fn decode(self) -> [u8; 3] {
        let c = self.0.get();
        [0xed, (c >> 8) as u8, c as u8]
    }
}

fn decode_surrogate_pair(high: HighSurrogate, low: LowSurrogate) -> [u8; 4] {
    // we want to transform the bits from:
    //
    //      high surrogate'   low surrogate
    //      101wvuts 10rqpnmk 1011jihg 10fedcba
    // to
    //      UTF-8
    //      11110wvu 10tsrqpn 10mkjihg 10fedcba
    // ...

    //       lo & 0xfff = 00000000 00000000 0000jihg 10fedbca
    //
    //         hi << 12 = 0000101w vuts10rq pnmk0000 00000000
    //   ... & 0x303000 = 00000000 00ts0000 00mk0000 00000000
    //
    //         hi << 14 = 00101wvu ts10rqpn mk000000 00000000
    //  ... & 0x70f0000 = 00000wvu 0000rqpn 00000000 00000000
    //
    //       0xf0808000 = 11110000 10000000 10000000 00000000
    //
    //        ... | ... = 11110wvu 10tsrqpn 10mkjihg 10fedcba
    let lo = low.0.get() as u32;
    let hi = (high.0.get() as u32) + 0x100;
    let combined = (lo & 0xfff) | (hi << 12 & 0x303000) | (hi << 14 & 0x70f0000) | 0xf0808000;
    unsafe { mem::transmute(u32::from_be(combined)) }
}

#[test]
fn test_decode_surrogate_pair() {
    fn check(hi: u16, lo: u16, utf8: [u8; 4]) {
        let high = HighSurrogate(NonZeroU16::new(hi).unwrap());
        let low = LowSurrogate(NonZeroU16::new(lo).unwrap());
        assert_eq!(decode_surrogate_pair(high, low), utf8);
    }
    check(0xa080, 0xb080, [0xf0, 0x90, 0x80, 0x80]);
    check(0xa0bd, 0xb88d, [0xf0, 0x9f, 0x98, 0x8d]);
    check(0xafbf, 0xbfbf, [0xf4, 0x8f, 0xbf, 0xbf]);
}


/// Represents a 3-byte sequence as part of a well-formed OMG-WTF-8 sequence.
///
/// Internally, the sequence is encoded as a big-endian integer to simplify
/// computation (not using native endian here since there's no advantage in
/// reading *3* bytes).
#[derive(Copy, Clone)]
struct ThreeByteSeq(u32);
impl ThreeByteSeq {
    fn to_high_surrogate_from_split_repr_unchecked(self) -> u16 {
        // the high surrogate in split representation has bit pattern
        //
        //  self.0 =        ******** 11110kji 10hgfedc 10ba****
        //
        // thus:
        //  self.0 >> 4 =   0000**** ****1111 0kji10hg fedc10ba
        //        0x303 =   00000000 00000000 00000011 00000011
        //            & =   00000000 00000000 000000hg 000000ba
        //
        //  self.0 >> 6 =   000000** ******11 110kji10 hgfedc10
        //       0x3c3c =   00000000 00000000 00111100 00111100
        //            & =   00000000 00000000 000kji00 00fedc00
        //
        //    ... | ... =   00000000 00000000 000kjihg 00fedcba
        //
        // The -0x100 is to account for the UTF-16 offset. The final
        // 0xa080 is to make the final bit patterns compare the same as
        // the canonical representation.
        //
        (((self.0 >> 4 & 0x303 | self.0 >> 6 & 0x3c3c) - 0x100) | 0xa080) as u16
    }

    /// Obtains the high surrogate value from this 3-byte sequence.
    ///
    /// If the input is not a high surrogate, returns None.
    fn to_high_surrogate(self) -> Option<HighSurrogate> {
        let surrogate_value = match self.0 {
            // canonical representation
            0xeda000..=0xedafff => self.0 as u16,
            // split representation
            0xf00000..=0xffffffff => self.to_high_surrogate_from_split_repr_unchecked(),
            _ => 0,
        };
        NonZeroU16::new(surrogate_value).map(HighSurrogate)
    }

    /// Obtains the low surrogate value from this 3-byte sequence.
    ///
    /// If the input is not a low surrogate, returns None.
    fn to_low_surrogate(self) -> Option<LowSurrogate> {
        let surrogate_value = match self.0 {
            // canonical representation
            0xedb000..=0xedffff => self.0,
            // split representation
            0x800000..=0xbfffff => self.0 | 0xb000,
            _ => 0,
        };
        NonZeroU16::new(surrogate_value as u16).map(LowSurrogate)
    }

    /// Extracts a WTF-16 code unit from the 3-byte sequence.
    fn as_code_unit(self) -> u16 {
        (match self.0 {
            0xf00000...0xffffffff => {
                (self.0 >> 4 & 3 | self.0 >> 6 & 0xfc | self.0 >> 8 & 0x700) + 0xd7c0
            }
            0x800000...0xbfffff => self.0 & 0x3f | self.0 >> 2 & 0x3c0 | 0xdc00,
            _ => self.0 & 0x3f | self.0 >> 2 & 0xfc0 | self.0 >> 4 & 0xf000,
        }) as u16
    }

    /// Constructs a 3-byte sequence from the bytes.
    fn new(input: &[u8]) -> Self {
        assert!(input.len() >= 3);
        ThreeByteSeq((input[0] as u32) << 16 | (input[1] as u32) << 8 | (input[2] as u32))
    }
}

/// An owned, growable string of well-formed WTF-8 data.
///
/// Similar to `String`, but can additionally contain surrogate code points
/// if they’re not in a surrogate pair.
#[derive(Eq, PartialEq, Ord, PartialOrd, Clone)]
pub struct Wtf8Buf {
    bytes: Vec<u8>
}

impl ops::Deref for Wtf8Buf {
    type Target = Wtf8;

    fn deref(&self) -> &Wtf8 {
        self.as_slice()
    }
}

impl ops::DerefMut for Wtf8Buf {
    fn deref_mut(&mut self) -> &mut Wtf8 {
        self.as_mut_slice()
    }
}

/// Format the string with double quotes,
/// and surrogates as `\u` followed by four hexadecimal digits.
/// Example: `"a\u{D800}"` for a string with code points [U+0061, U+D800]
impl fmt::Debug for Wtf8Buf {
    #[inline]
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl Wtf8Buf {
    /// Creates a new, empty WTF-8 string.
    #[inline]
    pub fn new() -> Wtf8Buf {
        Wtf8Buf { bytes: Vec::new() }
    }

    /// Creates a new, empty WTF-8 string with pre-allocated capacity for `n` bytes.
    #[inline]
    pub fn with_capacity(n: usize) -> Wtf8Buf {
        Wtf8Buf { bytes: Vec::with_capacity(n) }
    }

    /// Creates a WTF-8 string from a UTF-8 `String`.
    ///
    /// This takes ownership of the `String` and does not copy.
    ///
    /// Since WTF-8 is a superset of UTF-8, this always succeeds.
    #[inline]
    pub fn from_string(string: String) -> Wtf8Buf {
        Wtf8Buf { bytes: string.into_bytes() }
    }

    /// Creates a WTF-8 string from a UTF-8 `&str` slice.
    ///
    /// This copies the content of the slice.
    ///
    /// Since WTF-8 is a superset of UTF-8, this always succeeds.
    #[inline]
    pub fn from_str(str: &str) -> Wtf8Buf {
        Wtf8Buf { bytes: <[_]>::to_vec(str.as_bytes()) }
    }

    pub fn clear(&mut self) {
        self.bytes.clear()
    }

    /// Creates a WTF-8 string from a potentially ill-formed UTF-16 slice of 16-bit code units.
    ///
    /// This is lossless: calling `.encode_wide()` on the resulting string
    /// will always return the original code units.
    pub fn from_wide(v: &[u16]) -> Wtf8Buf {
        let mut string = Wtf8Buf::with_capacity(v.len());
        for item in char::decode_utf16(v.iter().cloned()) {
            match item {
                Ok(ch) => string.push_char(ch),
                Err(surrogate) => {
                    let surrogate = surrogate.unpaired_surrogate();
                    // Skip the WTF-8 concatenation check,
                    // surrogate pairs are already decoded by decode_utf16
                    string.push_code_point_unchecked(surrogate as u32)
                }
            }
        }
        string
    }

    /// Copied from String::push
    /// This does **not** include the WTF-8 concatenation check.
    fn push_code_point_unchecked(&mut self, code_point: u32) {
        let c = unsafe {
            char::from_u32_unchecked(code_point)
        };
        let mut bytes = [0; 4];
        let bytes = c.encode_utf8(&mut bytes).as_bytes();
        self.bytes.extend_from_slice(bytes)
    }

    #[inline]
    pub fn as_slice(&self) -> &Wtf8 {
        unsafe { Wtf8::from_bytes_unchecked(&self.bytes) }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut Wtf8 {
        unsafe { Wtf8::from_mut_bytes_unchecked(&mut self.bytes) }
    }

    /// Reserves capacity for at least `additional` more bytes to be inserted
    /// in the given `Wtf8Buf`.
    /// The collection may reserve more space to avoid frequent reallocations.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity overflows `usize`.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.bytes.reserve(additional)
    }

    #[inline]
    pub fn reserve_exact(&mut self, additional: usize) {
        self.bytes.reserve_exact(additional)
    }

    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.bytes.shrink_to_fit()
    }

    #[inline]
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.bytes.shrink_to(min_capacity)
    }

    /// Returns the number of bytes that this string buffer can hold without reallocating.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.bytes.capacity()
    }

    /// Append a UTF-8 slice at the end of the string.
    #[inline]
    pub fn push_str(&mut self, other: &str) {
        self.bytes.extend_from_slice(other.as_bytes())
    }

    /// Append a WTF-8 slice at the end of the string.
    ///
    /// This replaces newly paired surrogates at the boundary
    /// with a supplementary code point,
    /// like concatenating ill-formed UTF-16 strings effectively would.
    #[inline]
    pub fn push_wtf8(&mut self, other: &Wtf8) {
        self.bytes.reserve(other.len());
        let (low, mid, high) = other.canonicalize();
        if let Some(low) = low {
            self.push_low_surrogate(low);
            }
        self.bytes.extend_from_slice(mid);
        if let Some(high) = high {
            self.bytes.extend_from_slice(&high.decode());
        }
    }

    /// Append a Unicode scalar value at the end of the string.
    #[inline]
    pub fn push_char(&mut self, c: char) {
        self.push_code_point_unchecked(c as u32)
    }

    /// Append a low surrogate at the end of the string.
    fn push_low_surrogate(&mut self, trail: LowSurrogate) {
        if let Some(lead) = (&**self).split_off_last_high_surrogate() {
            // recombine the surrogate pair.
                let len_without_lead_surrogate = self.len() - 3;
                self.bytes.truncate(len_without_lead_surrogate);
            self.bytes.extend_from_slice(&decode_surrogate_pair(lead, trail));
        } else {
            // no matching surrogate pair, just push the low surrogate code unit.
            self.bytes.extend_from_slice(&trail.decode());
            }
        }

    /// Shortens a string to the specified length.
    ///
    /// # Panics
    ///
    /// Panics if `new_len` > current length,
    /// or if `new_len` is not a code point boundary.
    #[inline]
    pub fn truncate(&mut self, new_len: usize) {
        match classify_index(self, new_len) {
            IndexType::CharBoundary => {
                self.bytes.truncate(new_len);
            }
            IndexType::FourByteSeq2 => {
                self.bytes.truncate(new_len + 1);
                self.canonicalize_in_place();
            }
            _ => {
                panic!("not a code point boundary at index {}", new_len);
            }
        }
    }

    /// Consumes the WTF-8 string and tries to convert it to UTF-8.
    ///
    /// This does not copy the data.
    ///
    /// If the contents are not well-formed UTF-8
    /// (that is, if the string contains surrogates),
    /// the original WTF-8 string is returned instead.
    pub fn into_string(self) -> Result<String, Wtf8Buf> {
        match self.next_surrogate(0) {
            None => Ok(unsafe { String::from_utf8_unchecked(self.bytes) }),
            Some(_) => Err(self),
        }
    }

    /// Consumes the WTF-8 string and converts it lossily to UTF-8.
    ///
    /// This does not copy the data (but may overwrite parts of it in place).
    ///
    /// Surrogates are replaced with `"\u{FFFD}"` (the replacement character “�”)
    pub fn into_string_lossy(mut self) -> String {
        let mut pos = 0;
        loop {
            match self.next_surrogate(pos) {
                Some((surrogate_pos, _)) => {
                    pos = surrogate_pos + 3;
                    self.bytes[surrogate_pos..pos]
                        .copy_from_slice(UTF8_REPLACEMENT_CHARACTER.as_bytes());
                },
                None => return unsafe { String::from_utf8_unchecked(self.bytes) }
            }
        }
    }

    /// Converts this `Wtf8Buf` into a boxed `Wtf8`.
    #[inline]
    pub fn into_box(self) -> Box<Wtf8> {
        unsafe { mem::transmute(self.bytes.into_boxed_slice()) }
    }

    /// Converts a `Box<Wtf8>` into a `Wtf8Buf`.
    pub fn from_box(boxed: Box<Wtf8>) -> Wtf8Buf {
        let bytes: Box<[u8]> = unsafe { mem::transmute(boxed) };
        Wtf8Buf { bytes: bytes.into_vec() }
    }
}

/// A borrowed slice of well-formed WTF-8 data.
///
/// Similar to `&str`, but can additionally contain surrogate code points
/// if they’re not in a surrogate pair.
pub struct Wtf8 {
    bytes: [u8]
}

impl AsInner<[u8]> for Wtf8 {
    fn as_inner(&self) -> &[u8] { &self.bytes }
}

/// Format the slice with double quotes,
/// and surrogates as `\u` followed by four hexadecimal digits.
/// Example: `"a\u{D800}"` for a slice with code points [U+0061, U+D800]
impl fmt::Debug for Wtf8 {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        fn write_str_escaped(f: &mut fmt::Formatter, s: &str) -> fmt::Result {
            use fmt::Write;
            for c in s.chars().flat_map(|c| c.escape_debug()) {
                f.write_char(c)?
            }
            Ok(())
        }

        formatter.write_str("\"")?;
        let mut pos = 0;
        while let Some((surrogate_pos, surrogate)) = self.next_surrogate(pos) {
            write_str_escaped(
                formatter,
                unsafe { str::from_utf8_unchecked(
                    &self.bytes[pos .. surrogate_pos]
                )},
            )?;
            write!(formatter, "\\u{{{:x}}}", surrogate)?;
            pos = surrogate_pos + 3;
        }
        write_str_escaped(
            formatter,
            unsafe { str::from_utf8_unchecked(&self.bytes[pos..]) },
        )?;
        formatter.write_str("\"")
    }
}

impl fmt::Display for Wtf8 {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let wtf8_bytes = &self.bytes;
        let mut pos = 0;
        loop {
            match self.next_surrogate(pos) {
                Some((surrogate_pos, _)) => {
                    formatter.write_str(unsafe {
                        str::from_utf8_unchecked(&wtf8_bytes[pos .. surrogate_pos])
                    })?;
                    formatter.write_str(UTF8_REPLACEMENT_CHARACTER)?;
                    pos = surrogate_pos + 3;
                },
                None => {
                    let s = unsafe {
                        str::from_utf8_unchecked(&wtf8_bytes[pos..])
                    };
                    if pos == 0 {
                        return s.fmt(formatter)
                    } else {
                        return formatter.write_str(s)
                    }
                }
            }
        }
    }
}

impl Wtf8 {
    /// Creates a WTF-8 slice from a UTF-8 `&str` slice.
    ///
    /// Since WTF-8 is a superset of UTF-8, this always succeeds.
    #[inline]
    pub fn from_str(value: &str) -> &Wtf8 {
        unsafe { Wtf8::from_bytes_unchecked(value.as_bytes()) }
    }

    /// Creates a WTF-8 slice from a WTF-8 byte slice.
    ///
    /// Since the byte slice is not checked for valid WTF-8, this functions is
    /// marked unsafe.
    #[inline]
    unsafe fn from_bytes_unchecked(value: &[u8]) -> &Wtf8 {
        mem::transmute(value)
    }

    /// Creates a mutable WTF-8 slice from a mutable WTF-8 byte slice.
    ///
    /// Since the byte slice is not checked for valid WTF-8, this functions is
    /// marked unsafe.
    #[inline]
    unsafe fn from_mut_bytes_unchecked(value: &mut [u8]) -> &mut Wtf8 {
        mem::transmute(value)
    }

    /// Returns the length, in WTF-8 bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Returns the code point at `position` if it is in the ASCII range,
    /// or `b'\xFF' otherwise.
    ///
    /// # Panics
    ///
    /// Panics if `position` is beyond the end of the string.
    #[inline]
    pub fn ascii_byte_at(&self, position: usize) -> u8 {
        match self.bytes[position] {
            ascii_byte @ 0x00 ... 0x7F => ascii_byte,
            _ => 0xFF
        }
    }

    /// Tries to convert the string to UTF-8 and return a `&str` slice.
    ///
    /// Returns `None` if the string contains surrogates.
    ///
    /// This does not copy the data.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        // Well-formed WTF-8 is also well-formed UTF-8
        // if and only if it contains no surrogate.
        match self.next_surrogate(0) {
            None => Some(unsafe { str::from_utf8_unchecked(&self.bytes) }),
            Some(_) => None,
        }
    }

    /// Lossily converts the string to UTF-8.
    /// Returns a UTF-8 `&str` slice if the contents are well-formed in UTF-8.
    ///
    /// Surrogates are replaced with `"\u{FFFD}"` (the replacement character “�”).
    ///
    /// This only copies the data if necessary (if it contains any surrogate).
    pub fn to_string_lossy(&self) -> Cow<str> {
        let surrogate_pos = match self.next_surrogate(0) {
            None => return Cow::Borrowed(unsafe { str::from_utf8_unchecked(&self.bytes) }),
            Some((pos, _)) => pos,
        };
        let wtf8_bytes = &self.bytes;
        let mut utf8_bytes = Vec::with_capacity(self.len());
        utf8_bytes.extend_from_slice(&wtf8_bytes[..surrogate_pos]);
        utf8_bytes.extend_from_slice(UTF8_REPLACEMENT_CHARACTER.as_bytes());
        let mut pos = surrogate_pos + 3;
        loop {
            match self.next_surrogate(pos) {
                Some((surrogate_pos, _)) => {
                    utf8_bytes.extend_from_slice(&wtf8_bytes[pos .. surrogate_pos]);
                    utf8_bytes.extend_from_slice(UTF8_REPLACEMENT_CHARACTER.as_bytes());
                    pos = surrogate_pos + 3;
                },
                None => {
                    utf8_bytes.extend_from_slice(&wtf8_bytes[pos..]);
                    return Cow::Owned(unsafe { String::from_utf8_unchecked(utf8_bytes) })
                }
            }
        }
    }

    /// Converts the WTF-8 string to potentially ill-formed UTF-16
    /// and return an iterator of 16-bit code units.
    ///
    /// This is lossless:
    /// calling `Wtf8Buf::from_ill_formed_utf16` on the resulting code units
    /// would always return the original WTF-8 string.
    #[inline]
    pub fn encode_wide(&self) -> EncodeWide {
        EncodeWide { bytes: self.bytes.iter(), extra: 0 }
    }

    #[inline]
    fn next_surrogate(&self, mut pos: usize) -> Option<(usize, u16)> {
        loop {
            let inc = match *self.bytes.get(pos)? {
                0..=0x7f => 1,
                0x80..=0xbf => break,
                0xc0..=0xdf => 2,
                b @ 0xe0..=0xef => if b == 0xed && self.bytes[pos + 1] >= 0xa0 { break } else { 3 },
                0xf0..=0xff => if self.len() == pos + 3 { break } else { 4 },
                _ => unreachable!(),
            };
            pos += inc;
                    }
        Some((pos, ThreeByteSeq::new(&self.bytes[pos..]).as_code_unit()))
                }

    /// Splits-off the first low surrogate from the string.
    fn split_off_first_low_surrogate(self: &mut &Self) -> Option<LowSurrogate> {
        let input = self.bytes.get(..3)?;
        let res = ThreeByteSeq::new(input).to_low_surrogate()?;
        *self = unsafe { Self::from_bytes_unchecked(&self.bytes[3..]) };
        Some(res)
            }

    /// Splits-off the last high surrogate from the string.
    fn split_off_last_high_surrogate(self: &mut &Self) -> Option<HighSurrogate> {
        let e = self.len().checked_sub(3)?;
        let res = ThreeByteSeq::new(&self.bytes[e..]).to_high_surrogate()?;
        *self = unsafe { Self::from_bytes_unchecked(&self.bytes[..e]) };
        Some(res)
        }

    /// Split the string into three parts: the beginning low surrogate, the
    /// well-formed WTF-8 string in the middle, and the ending high surrogate.
    fn canonicalize(&self) -> (Option<LowSurrogate>, &[u8], Option<HighSurrogate>) {
        let mut s = self;
        let low = s.split_off_first_low_surrogate();
        let high = s.split_off_last_high_surrogate();
        (low, &s.bytes, high)
        }

    fn canonicalize_in_place(&mut self) {
        let len = self.len();
        if len < 3 {
            return;
        }
        // first 3 bytes form a low surrogate
        // (this check is a faster version of `(0x80..0xc0).contains(_)`).
        if (self.bytes[0] as i8) < -0x40 {
            self.bytes[0] = 0xed;
            self.bytes[1] |= 0x30;
        }
        // last 3 bytes form a high surrogate
        if self.bytes[len - 3] >= 0xf0 {
            let cu = ThreeByteSeq::new(&self.bytes[(len - 3)..])
                .to_high_surrogate_from_split_repr_unchecked();
            self.bytes[len - 3] = 0xed;
            self.bytes[len - 2] = (cu >> 8) as u8;
            self.bytes[len - 1] = cu as u8;
    }
    }

    /// Boxes this `Wtf8`.
    #[inline]
    pub fn into_box(&self) -> Box<Wtf8> {
        let boxed: Box<[u8]> = self.bytes.into();
        let mut res: Box<Wtf8> = unsafe { mem::transmute(boxed) };
        res.canonicalize_in_place();
        res
    }

    /// Creates a boxed, empty `Wtf8`.
    pub fn empty_box() -> Box<Wtf8> {
        let boxed: Box<[u8]> = Default::default();
        unsafe { mem::transmute(boxed) }
    }

    #[inline]
    pub fn into_arc(&self) -> Arc<Wtf8> {
        let arc: Arc<[u8]> = Arc::from(&self.bytes);
        let mut res = unsafe { Arc::from_raw(Arc::into_raw(arc) as *const Wtf8) };
        Arc::get_mut(&mut res).unwrap().canonicalize_in_place();
        res
    }

    #[inline]
    pub fn into_rc(&self) -> Rc<Wtf8> {
        let rc: Rc<[u8]> = Rc::from(&self.bytes);
        let mut res = unsafe { Rc::from_raw(Rc::into_raw(rc) as *const Wtf8) };
        Rc::get_mut(&mut res).unwrap().canonicalize_in_place();
        res
    }
}

// FIXME: Comparing Option<Surrogate> is not fully optimized yet #49892.

impl PartialEq for Wtf8 {
    fn eq(&self, other: &Self) -> bool {
        self.canonicalize() == other.canonicalize()
    }
    fn ne(&self, other: &Self) -> bool {
        self.canonicalize() != other.canonicalize()
    }
}
impl Eq for Wtf8 {}

impl PartialOrd for Wtf8 {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        self.canonicalize().partial_cmp(&other.canonicalize())
    }
    fn lt(&self, other: &Self) -> bool {
        self.canonicalize() < other.canonicalize()
    }
    fn le(&self, other: &Self) -> bool {
        self.canonicalize() <= other.canonicalize()
    }
    fn gt(&self, other: &Self) -> bool {
        self.canonicalize() > other.canonicalize()
    }
    fn ge(&self, other: &Self) -> bool {
        self.canonicalize() >= other.canonicalize()
    }
}
impl Ord for Wtf8 {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.canonicalize().cmp(&other.canonicalize())
    }
}

/// Return a slice of the given string for the byte range [`begin`..`end`).
///
/// # Panics
///
/// Panics when `begin` and `end` do not point to code point boundaries,
/// or point beyond the end of the string.
impl ops::Index<ops::Range<usize>> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    fn index(&self, mut range: ops::Range<usize>) -> &Wtf8 {
        if range.start == range.end {
            return Self::from_str("");
        }
        match classify_index(self, range.start) {
            IndexType::FourByteSeq2 => range.start -= 1,
            IndexType::CharBoundary => {}
            _ => slice_error_fail(self, range.start, range.end),
        };
        match classify_index(self, range.end) {
            IndexType::FourByteSeq2 => range.end += 1,
            IndexType::CharBoundary => {}
            _ => slice_error_fail(self, range.start, range.end),
        };
        unsafe { slice_unchecked(self, range.start, range.end) }
    }
}

/// Return a slice of the given string from byte `begin` to its end.
///
/// # Panics
///
/// Panics when `begin` is not at a code point boundary,
/// or is beyond the end of the string.
impl ops::Index<ops::RangeFrom<usize>> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    fn index(&self, mut range: ops::RangeFrom<usize>) -> &Wtf8 {
        match classify_index(self, range.start) {
            IndexType::FourByteSeq2 => range.start -= 1,
            IndexType::CharBoundary => {}
            _ => slice_error_fail(self, range.start, self.len()),
        };
        unsafe { slice_unchecked(self, range.start, self.len()) }
    }
}

/// Return a slice of the given string from its beginning to byte `end`.
///
/// # Panics
///
/// Panics when `end` is not at a code point boundary,
/// or is beyond the end of the string.
impl ops::Index<ops::RangeTo<usize>> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    fn index(&self, mut range: ops::RangeTo<usize>) -> &Wtf8 {
        match classify_index(self, range.end) {
            IndexType::FourByteSeq2 => range.end += 1,
            IndexType::CharBoundary => {}
            _ => slice_error_fail(self, 0, range.end),
        };
            unsafe { slice_unchecked(self, 0, range.end) }
    }
}

impl ops::Index<ops::RangeFull> for Wtf8 {
    type Output = Wtf8;

    #[inline]
    fn index(&self, _range: ops::RangeFull) -> &Wtf8 {
        self
    }
}

/// Type of an index in an OMG-WTF-8 string.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
enum IndexType {
    /// Boundary of a WTF-8 character sequence.
    CharBoundary = 0,
    /// Byte 1 in a 4-byte sequence.
    FourByteSeq1 = 1,
    /// Byte 2 in a 4-byte sequence.
    FourByteSeq2 = 2,
    /// Byte 3 in a 4-byte sequence.
    FourByteSeq3 = 3,
    /// Pointing inside a 2- or 3-byte sequence.
    Interior = 4,
    /// Out of bounds.
    OutOfBounds = 5,
}

/// Classifies the kind of index in this string.
fn classify_index(slice: &Wtf8, index: usize) -> IndexType {
    let slice = &slice.bytes;
    let len = slice.len();
    if index == 0 || index == len {
        return IndexType::CharBoundary;
    }
    match slice.get(index) {
        Some(0x80..=0xbf) => {
            let max_offset = index.min(3);
            let min_offset = (index + 3).saturating_sub(len);
            for offset in min_offset..max_offset {
                let offset = offset + 1;
                unsafe {
                    if slice.get_unchecked(index - offset) >= &0xf0 {
                        return mem::transmute(offset as u8);
                    }
                }
            }
            IndexType::Interior
        }
        Some(_) => IndexType::CharBoundary,
        None => IndexType::OutOfBounds,
    }
}

/// Copied from core::str::raw::slice_unchecked
#[inline]
pub unsafe fn slice_unchecked(s: &Wtf8, begin: usize, end: usize) -> &Wtf8 {
    // memory layout of an &[u8] and &Wtf8 are the same
    Wtf8::from_bytes_unchecked(slice::from_raw_parts(
        s.bytes.as_ptr().offset(begin as isize),
        end - begin
    ))
}

/// Copied from core::str::raw::slice_error_fail
#[inline(never)]
pub fn slice_error_fail(s: &Wtf8, begin: usize, end: usize) -> ! {
    assert!(begin <= end);
    panic!("index {} and/or {} in `{:?}` do not lie on character boundary",
          begin, end, s);
}

/// Generates a wide character sequence for potentially ill-formed UTF-16.
#[stable(feature = "rust1", since = "1.0.0")]
#[derive(Clone)]
pub struct EncodeWide<'a> {
    bytes: slice::Iter<'a, u8>,
    extra: u16,
}

// Copied from libunicode/u_str.rs
#[stable(feature = "rust1", since = "1.0.0")]
impl<'a> Iterator for EncodeWide<'a> {
    type Item = u16;

    #[inline]
    fn next(&mut self) -> Option<u16> {
        if self.extra != 0 {
            let tmp = self.extra;
            self.extra = 0;
            return Some(tmp);
        }

        let sl = self.bytes.as_slice();
        let is_split_surrogate = match *sl.get(0)? {
            0x80..=0xbf => true,
            0xf0..=0xff if sl.len() == 3 => true,
            _ => false,
        };

        if is_split_surrogate {
            self.bytes.next();
            self.bytes.next();
            self.bytes.next();
            Some(ThreeByteSeq::new(sl).as_code_unit())
        } else {
            let code_point = next_code_point(&mut self.bytes)?;
            let c = unsafe { char::from_u32_unchecked(code_point) };
        let mut buf = [0; 2];
            let n = c.encode_utf16(&mut buf).len();
            if n == 2 {
                self.extra = buf[1];
            }
            Some(buf[0])
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        // converting from WTF-8 to WTF-16:
        //  1-byte seq => 1 code unit (1x)
        //  2-byte seq => 1 code unit (0.5x)
        //  3-byte seq => 1 code unit (0.33x)
        //  4-byte seq => 2 code units (0.5x)
        //
        // thus the lower-limit is everything being a 3-byte seq (= ceil(len/3))
        // and upper-limit is everything being 1-byte seq (= len).
        let len = self.bytes.len();
        (len.saturating_add(2) / 3, Some(len))
    }
}

impl Hash for Wtf8Buf {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(&self.bytes);
        0xfeu8.hash(state)
    }
}

impl Hash for Wtf8 {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (left, middle, right) = self.canonicalize();
        if let Some(low) = left {
            state.write(&low.decode());
        }
        state.write(middle);
        if let Some(high) = right {
            state.write(&high.decode());
        }
        0xfeu8.hash(state)
    }
}

impl Wtf8 {
    pub fn make_ascii_uppercase(&mut self) { self.bytes.make_ascii_uppercase() }
}

#[cfg(test)]
mod tests {
    use borrow::Cow;
    use super::*;

    #[test]
    fn wtf8buf_new() {
        assert_eq!(Wtf8Buf::new().bytes, b"");
    }

    #[test]
    fn wtf8buf_from_str() {
        assert_eq!(Wtf8Buf::from_str("").bytes, b"");
        assert_eq!(Wtf8Buf::from_str("aé 💩").bytes,
                   b"a\xC3\xA9 \xF0\x9F\x92\xA9");
    }

    #[test]
    fn wtf8buf_from_string() {
        assert_eq!(Wtf8Buf::from_string(String::from("")).bytes, b"");
        assert_eq!(Wtf8Buf::from_string(String::from("aé 💩")).bytes,
                   b"a\xC3\xA9 \xF0\x9F\x92\xA9");
    }

    #[test]
    fn wtf8buf_from_wide() {
        assert_eq!(Wtf8Buf::from_wide(&[]).bytes, b"");
        assert_eq!(Wtf8Buf::from_wide(
                      &[0x61, 0xE9, 0x20, 0xD83D, 0xD83D, 0xDCA9]).bytes,
                   b"a\xC3\xA9 \xED\xA0\xBD\xF0\x9F\x92\xA9");
    }

    #[test]
    fn wtf8buf_push_str() {
        let mut string = Wtf8Buf::new();
        assert_eq!(string.bytes, b"");
        string.push_str("aé 💩");
        assert_eq!(string.bytes, b"a\xC3\xA9 \xF0\x9F\x92\xA9");
    }

    #[test]
    fn wtf8buf_push_char() {
        let mut string = Wtf8Buf::from_str("aé ");
        assert_eq!(string.bytes, b"a\xC3\xA9 ");
        string.push_char('💩');
        assert_eq!(string.bytes, b"a\xC3\xA9 \xF0\x9F\x92\xA9");
    }

    #[test]
    fn wtf8buf_push() {
        let mut string = Wtf8Buf::from_str("aé ");
        assert_eq!(string.bytes, b"a\xC3\xA9 ");
        string.push_char('💩');
        assert_eq!(string.bytes, b"a\xC3\xA9 \xF0\x9F\x92\xA9");

        let l = LowSurrogate::from_code_point_unchecked;

        let mut string = Wtf8Buf::new();
        string.push_code_point_unchecked(0xD83D);   // lead
        string.push_low_surrogate(l(0xDCA9));       // trail
        assert_eq!(string.bytes, b"\xF0\x9F\x92\xA9");  // Magic!

        let mut string = Wtf8Buf::new();
        string.push_code_point_unchecked(0xD83D);   // lead
        string.push_code_point_unchecked(0x20);     // not surrogate
        string.push_low_surrogate(l(0xDCA9));       // trail
        assert_eq!(string.bytes, b"\xED\xA0\xBD \xED\xB2\xA9");

        let mut string = Wtf8Buf::new();
        string.push_code_point_unchecked(0xD7FF);   // not surrogate
        string.push_low_surrogate(l(0xDC00));       // trail
        assert_eq!(string.bytes, b"\xED\x9F\xBF\xED\xB0\x80");

        let mut string = Wtf8Buf::new();
        string.push_code_point_unchecked(0x61);     // not surrogate, < 3 bytes
        string.push_low_surrogate(l(0xDC00));       // trail
        assert_eq!(string.bytes, b"\x61\xED\xB0\x80");

        let mut string = Wtf8Buf::new();
        string.push_low_surrogate(l(0xDC00));       // trail
        assert_eq!(string.bytes, b"\xED\xB0\x80");
    }

    #[test]
    fn wtf8buf_push_wtf8() {
        let mut string = Wtf8Buf::from_str("aé");
        assert_eq!(string.bytes, b"a\xC3\xA9");
        string.push_wtf8(Wtf8::from_str(" 💩"));
        assert_eq!(string.bytes, b"a\xC3\xA9 \xF0\x9F\x92\xA9");

        fn w(v: &[u8]) -> &Wtf8 { unsafe { Wtf8::from_bytes_unchecked(v) } }

        let mut string = Wtf8Buf::new();
        string.push_wtf8(w(b"\xED\xA0\xBD"));  // lead
        string.push_wtf8(w(b"\xED\xB2\xA9"));  // trail
        assert_eq!(string.bytes, b"\xF0\x9F\x92\xA9");  // Magic!

        let mut string = Wtf8Buf::new();
        string.push_wtf8(w(b"\xED\xA0\xBD"));  // lead
        string.push_wtf8(w(b" "));  // not surrogate
        string.push_wtf8(w(b"\xED\xB2\xA9"));  // trail
        assert_eq!(string.bytes, b"\xED\xA0\xBD \xED\xB2\xA9");

        let mut string = Wtf8Buf::new();
        string.push_wtf8(w(b"\xED\xA0\x80"));  // lead
        string.push_wtf8(w(b"\xED\xAF\xBF"));  // lead
        assert_eq!(string.bytes, b"\xED\xA0\x80\xED\xAF\xBF");

        let mut string = Wtf8Buf::new();
        string.push_wtf8(w(b"\xED\xA0\x80"));  // lead
        string.push_wtf8(w(b"\xEE\x80\x80"));  // not surrogate
        assert_eq!(string.bytes, b"\xED\xA0\x80\xEE\x80\x80");

        let mut string = Wtf8Buf::new();
        string.push_wtf8(w(b"\xED\x9F\xBF"));  // not surrogate
        string.push_wtf8(w(b"\xED\xB0\x80"));  // trail
        assert_eq!(string.bytes, b"\xED\x9F\xBF\xED\xB0\x80");

        let mut string = Wtf8Buf::new();
        string.push_wtf8(w(b"a"));  // not surrogate, < 3 bytes
        string.push_wtf8(w(b"\xED\xB0\x80"));  // trail
        assert_eq!(string.bytes, b"\x61\xED\xB0\x80");

        let mut string = Wtf8Buf::new();
        string.push_wtf8(w(b"\xED\xB0\x80"));  // trail
        assert_eq!(string.bytes, b"\xED\xB0\x80");
    }

    #[test]
    fn wtf8buf_truncate() {
        let mut string = Wtf8Buf::from_str("aé");
        string.truncate(1);
        assert_eq!(string.bytes, b"a");

        let mut string = Wtf8Buf::from_str("\u{10000}");
        string.truncate(2);
        assert_eq!(string.bytes, b"\xed\xa0\x80");
    }

    #[test]
    #[should_panic]
    fn wtf8buf_truncate_fail_code_point_boundary() {
        let mut string = Wtf8Buf::from_str("aé");
        string.truncate(2);
    }

    #[test]
    #[should_panic]
    fn wtf8buf_truncate_fail_4_byte_seq_interior_1() {
        let mut string = Wtf8Buf::from_str("\u{10000}");
        string.truncate(1);
    }

    #[test]
    #[should_panic]
    fn wtf8buf_truncate_fail_4_byte_seq_interior_3() {
        let mut string = Wtf8Buf::from_str("\u{10000}");
        string.truncate(3);
    }

    #[test]
    #[should_panic]
    fn wtf8buf_truncate_fail_longer() {
        let mut string = Wtf8Buf::from_str("aé");
        string.truncate(4);
    }

    #[test]
    fn wtf8buf_into_string() {
        let mut string = Wtf8Buf::from_str("aé 💩");
        assert_eq!(string.clone().into_string(), Ok(String::from("aé 💩")));
        string.push_code_point_unchecked(0xDC00);
        string.push_code_point_unchecked(0xD800);
        assert_eq!(string.clone().into_string(), Err(string));
    }

    #[test]
    fn wtf8buf_into_string_lossy() {
        let mut string = Wtf8Buf::from_str("aé 💩");
        assert_eq!(string.clone().into_string_lossy(), String::from("aé 💩"));
        string.push_code_point_unchecked(0xDC00);
        string.push_code_point_unchecked(0xD800);
        assert_eq!(string.clone().into_string_lossy(), String::from("aé 💩��"));
    }

    #[test]
    fn wtf8buf_show() {
        let mut string = Wtf8Buf::from_str("a\té \u{7f}💩\r");
        string.push_code_point_unchecked(0xDC00);
        string.push_code_point_unchecked(0xD800);
        assert_eq!(format!("{:?}", string), "\"a\\té \\u{7f}\u{1f4a9}\\r\\u{dc00}\\u{d800}\"");
    }

    #[test]
    fn wtf8buf_as_slice() {
        assert_eq!(Wtf8Buf::from_str("aé").as_slice(), Wtf8::from_str("aé"));
    }

    #[test]
    fn wtf8buf_show_str() {
        let text = "a\té 💩\r";
        let string = Wtf8Buf::from_str(text);
        assert_eq!(format!("{:?}", text), format!("{:?}", string));
    }

    #[test]
    fn wtf8_from_str() {
        assert_eq!(&Wtf8::from_str("").bytes, b"");
        assert_eq!(&Wtf8::from_str("aé 💩").bytes, b"a\xC3\xA9 \xF0\x9F\x92\xA9");
    }

    #[test]
    fn wtf8_len() {
        assert_eq!(Wtf8::from_str("").len(), 0);
        assert_eq!(Wtf8::from_str("aé 💩").len(), 8);
    }

    #[test]
    fn wtf8_slice() {
        assert_eq!(&Wtf8::from_str("aé 💩")[1.. 4].bytes, b"\xC3\xA9 ");
    }

    #[test]
    fn omgwtf8_slice() {
        let s = Wtf8::from_str("😀😂😄");
        assert_eq!(&s[..].bytes, b"\xf0\x9f\x98\x80\xf0\x9f\x98\x82\xf0\x9f\x98\x84");
        assert_eq!(&s[2..].bytes, b"\x9f\x98\x80\xf0\x9f\x98\x82\xf0\x9f\x98\x84");
        assert_eq!(&s[4..].bytes, b"\xf0\x9f\x98\x82\xf0\x9f\x98\x84");
        assert_eq!(&s[..10].bytes, b"\xf0\x9f\x98\x80\xf0\x9f\x98\x82\xf0\x9f\x98");
        assert_eq!(&s[..8].bytes, b"\xf0\x9f\x98\x80\xf0\x9f\x98\x82");
        assert_eq!(&s[2..10].bytes, b"\x9f\x98\x80\xf0\x9f\x98\x82\xf0\x9f\x98");
        assert_eq!(&s[4..8].bytes, b"\xf0\x9f\x98\x82");
        assert_eq!(&s[2..4].bytes, b"\x9f\x98\x80");
        assert_eq!(&s[2..2].bytes, b"");
        assert_eq!(&s[0..2].bytes, b"\xf0\x9f\x98");
        assert_eq!(&s[4..4].bytes, b"");
    }

    #[test]
    #[should_panic]
    fn wtf8_slice_not_code_point_boundary() {
        &Wtf8::from_str("aé 💩")[2.. 4];
    }

    #[test]
    fn wtf8_slice_from() {
        assert_eq!(&Wtf8::from_str("aé 💩")[1..].bytes, b"\xC3\xA9 \xF0\x9F\x92\xA9");
    }

    #[test]
    #[should_panic]
    fn wtf8_slice_from_not_code_point_boundary() {
        &Wtf8::from_str("aé 💩")[2..];
    }

    #[test]
    fn wtf8_slice_to() {
        assert_eq!(&Wtf8::from_str("aé 💩")[..4].bytes, b"a\xC3\xA9 ");
    }

    #[test]
    #[should_panic]
    fn wtf8_slice_to_not_code_point_boundary() {
        &Wtf8::from_str("aé 💩")[5..];
    }

    #[test]
    #[should_panic]
    fn test_slice_into_invalid_index_split_begin_1() {
        let s = unsafe { Wtf8::from_bytes_unchecked(b"\x90\x80\x80\x7e") };
        let _ = s[..1];
    }
    #[test]
    #[should_panic]
    fn test_slice_into_invalid_index_split_begin_2() {
        let s = unsafe { Wtf8::from_bytes_unchecked(b"\x90\x80\x80\x7e") };
        let _ = s[..2];
    }
    #[test]
    #[should_panic]
    fn test_slice_into_invalid_index_split_end_1() {
        let s = unsafe { Wtf8::from_bytes_unchecked(b"\x7e\xf0\x90\x80") };
        let _ = s[2..];
    }
    #[test]
    #[should_panic]
    fn test_slice_into_invalid_index_split_end_2() {
        let s = unsafe { Wtf8::from_bytes_unchecked(b"\x7e\xf0\x90\x80") };
        let _ = s[3..];
    }
    #[test]
    #[should_panic]
    fn test_slice_into_invalid_index_canonical_1() {
        let s = unsafe { Wtf8::from_bytes_unchecked(b"\xed\xaf\xbf") };
        let _ = s[1..];
    }
    #[test]
    #[should_panic]
    fn test_slice_into_invalid_index_canonical_2() {
        let s = unsafe { Wtf8::from_bytes_unchecked(b"\xed\xaf\xbf") };
        let _ = s[2..];
    }
    #[test]
    #[should_panic]
    fn test_slice_into_invalid_index_wrong_order() {
        let s = Wtf8::from_str("12345");
        let _ = s[3..1];
    }

    #[test]
    fn wtf8_ascii_byte_at() {
        let slice = Wtf8::from_str("aé 💩");
        assert_eq!(slice.ascii_byte_at(0), b'a');
        assert_eq!(slice.ascii_byte_at(1), b'\xFF');
        assert_eq!(slice.ascii_byte_at(2), b'\xFF');
        assert_eq!(slice.ascii_byte_at(3), b' ');
        assert_eq!(slice.ascii_byte_at(4), b'\xFF');
    }

    #[test]
    fn wtf8_as_str() {
        assert_eq!(Wtf8::from_str("").as_str(), Some(""));
        assert_eq!(Wtf8::from_str("aé 💩").as_str(), Some("aé 💩"));
        let mut string = Wtf8Buf::new();
        string.push_code_point_unchecked(0xD800);
        assert_eq!(string.as_str(), None);
    }

    #[test]
    fn wtf8_to_string_lossy() {
        assert_eq!(Wtf8::from_str("").to_string_lossy(), Cow::Borrowed(""));
        assert_eq!(Wtf8::from_str("aé 💩").to_string_lossy(), Cow::Borrowed("aé 💩"));
        let mut string = Wtf8Buf::from_str("aé 💩");
        string.push_code_point_unchecked(0xD800);
        let expected: Cow<str> = Cow::Owned(String::from("aé 💩�"));
        assert_eq!(string.to_string_lossy(), expected);
    }

    #[test]
    fn wtf8_display() {
        fn d(b: &[u8]) -> String {
            format!("{}", &unsafe { Wtf8::from_bytes_unchecked(b) })
        }

        assert_eq!("", d("".as_bytes()));
        assert_eq!("aé 💩", d("aé 💩".as_bytes()));

        let mut string = Wtf8Buf::from_str("aé 💩");
        string.push_code_point_unchecked(0xD800);
        assert_eq!("aé 💩�", d(string.as_inner()));
    }

    #[test]
    fn wtf8_encode_wide() {
        let mut string = Wtf8Buf::from_str("aé ");
        string.push_code_point_unchecked(0xD83D);
        string.push_char('💩');
        assert_eq!(string.encode_wide().collect::<Vec<_>>(),
                   vec![0x61, 0xE9, 0x20, 0xD83D, 0xD83D, 0xDCA9]);
    }

    #[test]
    fn omgwtf8_encode_wide() {
        let s = Wtf8::from_str("😀😂😄");
        assert_eq!(
            s.encode_wide().collect::<Vec<_>>(),
            vec![0xd83d, 0xde00, 0xd83d, 0xde02, 0xd83d, 0xde04]
        );
        assert_eq!(
            s[2..].encode_wide().collect::<Vec<_>>(),
            vec![0xde00, 0xd83d, 0xde02, 0xd83d, 0xde04]
        );
        assert_eq!(
            s[..10].encode_wide().collect::<Vec<_>>(),
            vec![0xd83d, 0xde00, 0xd83d, 0xde02, 0xd83d]
        );
    }

    #[test]
    fn omgwtf8_eq_hash() {
        use collections::hash_map::DefaultHasher;

        let a = unsafe { Wtf8::from_bytes_unchecked(b"\x90\x8b\xae~\xf0\x90\x80") };
        let b = unsafe { Wtf8::from_bytes_unchecked(b"\xed\xbb\xae~\xf0\x90\x80") };
        let c = unsafe { Wtf8::from_bytes_unchecked(b"\x90\x8b\xae~\xed\xa0\x80") };
        let d = unsafe { Wtf8::from_bytes_unchecked(b"\xed\xbb\xae~\xed\xa0\x80") };
        let e = Wtf8Buf::from_box(a.into_box());

        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(c, d);
        assert_eq!(d, &*e);

        fn hash<H: Hash>(a: H) -> u64 {
            let mut h = DefaultHasher::new();
            a.hash(&mut h);
            h.finish()
        }

        assert_eq!(hash(a), hash(b));
        assert_eq!(hash(b), hash(c));
        assert_eq!(hash(c), hash(d));
        assert_eq!(hash(d), hash(e));
    }

    #[test]
    fn omgwtf8_classify_index() {
        use super::IndexType::*;

        fn check(input: &Wtf8, expected: &[IndexType]) {
            let actual = (0..expected.len()).map(|i| classify_index(input, i)).collect::<Vec<_>>();
            assert_eq!(&*actual, expected);
        }
        check(
            Wtf8::from_str(""),
            &[CharBoundary, OutOfBounds, OutOfBounds],
        );
        check(
            Wtf8::from_str("aa"),
            &[CharBoundary, CharBoundary, CharBoundary, OutOfBounds],
        );
        check(
            Wtf8::from_str("á"),
            &[CharBoundary, Interior, CharBoundary, OutOfBounds],
        );
        check(
            Wtf8::from_str("\u{3000}"),
            &[CharBoundary, Interior, Interior, CharBoundary, OutOfBounds],
        );
        check(
            Wtf8::from_str("\u{30000}"),
            &[CharBoundary, FourByteSeq1, FourByteSeq2, FourByteSeq3, CharBoundary, OutOfBounds],
        );
        check(
            unsafe { Wtf8::from_bytes_unchecked(b"\xed\xbf\xbf\xed\xa0\x80") },
            &[
                CharBoundary, Interior, Interior,
                CharBoundary, Interior, Interior,
                CharBoundary, OutOfBounds,
            ],
        );
        check(
            unsafe { Wtf8::from_bytes_unchecked(b"\x90\x80\x80\xf0\x90\x80\x80\xf0\x90\x80") },
            &[
                CharBoundary, Interior, Interior,
                CharBoundary, FourByteSeq1, FourByteSeq2, FourByteSeq3,
                CharBoundary, Interior, Interior,
                CharBoundary, OutOfBounds,
            ],
        );
    }
}
