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

#![feature(inline_semantic)]

#[inline(semantic)]
fn factorial(x: u64) -> u64 { //~ ERROR: `#[inline(semantic)]` function cannot call itself.
    if x <= 1 {
        1
    } else {
        x * factorial(x - 1) //~ NOTE: recursion here
    }
}

fn main() {
    assert_eq!(120, factorial(5));
}