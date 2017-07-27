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

#![feature(implicit_caller_location, panic_col, caller_location)]

extern crate core;

use std::panic::*;

#[implicit_caller_location]
fn location() -> (&'static str, u32, u32) {
    let loc = Location::caller();
    (loc.file(), loc.line(), loc.column())
}

#[implicit_caller_location]
fn recursion(a: u32) -> u32 {
    if a > 0 {
        Location::caller().line() + recursion(a - 1)
    } else {
        0
    }
}

struct S<T>(Option<T>);

impl<T> S<T> {
    #[implicit_caller_location]
    fn unwrap(self) -> Result<T, u32> {
        match self.0 {
            Some(x) => Ok(x),
            None => Err(Location::caller().line()),
        }
    }

    #[implicit_caller_location]
    fn with_closure<R, F: FnOnce(Option<T>, u32) -> R>(self, f: F) -> R {
        f(self.0, Location::caller().line())
    }
}

fn assert_panic_location<F>(f: F, file: &'static str, line: u32, column: u32)
where
    F: (FnOnce() -> u32) + UnwindSafe
{
    let r = catch_unwind(move || {
        set_hook(Box::new(move |info| {
            let location = info.location().unwrap();
            println!("F: {} == {}?", location.file(), file);
            println!("L: {} == {}?", location.line(), line);
            println!("C: {} == {}?", location.column(), column);
            assert_eq!((location.file(), location.line(), location.column()), (file, line, column));
        }));
        f();
    });
    take_hook();
    r.unwrap_err();
}

fn main() {
    assert_eq!((file!(), line!(), 39), location());
    assert_eq!((file!(), line!(), 39), location());
    assert_eq!((file!(), line!(), 39), location());
    assert_eq!((file!(), line!(), 39), location());

    let fptr = location;
    let a = fptr();
    assert_eq!(a, fptr());
    assert_eq!(a, fptr());
    assert_eq!(a, fptr());
    assert_ne!(a.1, line!());

    assert_eq!(location(), (file!(), line!(), 15));

    assert_eq!(S(Some('a')).unwrap(), Ok('a'));
    assert_eq!(S(None::<char>).unwrap(), Err(line!()));

    assert_eq!(
        line!() + 1,
        S(Some("?"))
            .with_closure(
                |_, line| {
                    line
                }
            )
    );

    assert_eq!(recursion(10), line!() * 10);

    assert_panic_location(|| panic!(), file!(), line!(), 29);
    assert_panic_location(|| None::<u32>.unwrap(), file!(), line!(), 29);
    assert_panic_location(|| None::<u32>.expect("..."), file!(), line!(), 29);
    assert_panic_location(|| Ok::<_, u32>(1).unwrap_err(), file!(), line!(), 29);
    assert_panic_location(|| Ok::<_, u32>(1).expect_err("..."), file!(), line!(), 29);
    assert_panic_location(|| Err::<u32, _>(1).unwrap(), file!(), line!(), 29);
    assert_panic_location(|| Err::<u32, _>(1).expect("..."), file!(), line!(), 29);
}
