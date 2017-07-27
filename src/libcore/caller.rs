// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![unstable(feature = "caller_location", issue = "99999")]

//! Caller location information
//!
//! Used together with `#[inline(semantic)]` functions. The constants in this
//! module allows `#[inline(semantic)]` functions to know where it is called.

/// The file name of the caller of a semantically-inlined function.
///
/// This static variable can only be used inside a function with attribute
/// `#[inline(semantic)]`. When the function is successfully inlined, it will be
/// replaced by the file name at the original call site (as reported by
/// `file!()`).
#[cfg_attr(not(stage0), lang = "caller_file")]
pub const FILE: &str = "<dynamic>";

/// The line number of the caller of a semantically-inlined function.
///
/// This static variable can only be used inside a function with attribute
/// `#[inline(semantic)]`. When the function is successfully inlined, it will be
/// replaced by the line number at the original call site (as reported by
/// `line!()`).
#[cfg_attr(not(stage0), lang = "caller_line")]
pub const LINE: u32 = 0;

/// The column number of the caller of a semantically-inlined function.
///
/// This static variable can only be used inside a function with attribute
/// `#[inline(semantic)]`. When the function is successfully inlined, it will be
/// replaced by the column number at the original call site (as reported by
/// `column!()`).
#[cfg_attr(not(stage0), lang = "caller_column")]
pub const COLUMN: u32 = 0;
