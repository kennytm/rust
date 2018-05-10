// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! rustbuild, the Rust build system
//!
//! This is the entry point for the build system used to compile the `rustc`
//! compiler. Lots of documentation can be found in the `README.md` file in the
//! parent directory, and otherwise documentation can be found throughout the `build`
//! directory in each respective module.

#![deny(warnings)]

extern crate bootstrap;

use std::env;

use bootstrap::{Config, Build};

fn main() {
    if true {
        type UINT = u32;
        const SEM_NOGPFAULTERRORBOX: UINT = 0x0002;
        extern "system" {
            fn SetErrorMode(mode: UINT) -> UINT;
        }
        unsafe {
            let mode = SetErrorMode(0);
            SetErrorMode(mode & !SEM_NOGPFAULTERRORBOX);
        }

        let g = unsafe { std::ptr::read_volatile(4usize as *const u8) };
        assert_eq!(g, 123);
    }
    let args = env::args().skip(1).collect::<Vec<_>>();
    let config = Config::parse(&args);
    Build::new(config).build();
}
