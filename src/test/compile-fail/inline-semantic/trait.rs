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

trait Trait {
    fn foo(&self);
}

impl Trait for u32 {
    #[inline(semantic)] //~ ERROR: `#[inline(semantic)]` is not supported for trait items yet
    fn foo(&self) {
    }
}

fn main() {
    1u32.foo();
}