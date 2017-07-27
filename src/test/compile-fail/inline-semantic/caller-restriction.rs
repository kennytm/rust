// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// ignore-stage0

#![feature(inline_semantic, caller_location)]

extern crate core;
use core::caller::LINE;

static L: u32 = LINE; //~ ERROR: Cannot read caller location outside of `#[inline(semantic)]` function

#[inline(always)]
fn not_inline_semantic() -> u32 {
    LINE //~  ERROR: Cannot read caller location outside of `#[inline(semantic)]` function
}

#[inline(semantic)]
fn inline_semantic() -> u32 {
    const L: u32 = LINE; //~  ERROR: Cannot read caller location outside of `#[inline(semantic)]` function
    let closure = || LINE; //~  ERROR: Cannot read caller location outside of `#[inline(semantic)]` function
    LINE // only this one is ok.
}

fn main() {
    let _ = L;
    let _ = LINE; //~ ERROR: Cannot read caller location outside of `#[inline(semantic)]` function
    not_inline_semantic();
    inline_semantic();
}
