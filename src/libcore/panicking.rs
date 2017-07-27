// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Panic support for libcore
//!
//! The core library cannot define panicking, but it does *declare* panicking. This
//! means that the functions inside of libcore are allowed to panic, but to be
//! useful an upstream crate must define panicking for libcore to use. The current
//! interface for panicking is:
//!
//! ```
//! # use std::fmt;
//! fn panic_impl(fmt: fmt::Arguments, file_line_col: &(&'static str, u32, u32)) -> !
//! # { loop {} }
//! ```
//!
//! This definition allows for panicking with any general message, but it does not
//! allow for failing with a `Box<Any>` value. The reason for this is that libcore
//! is not allowed to allocate.
//!
//! This module contains a few other panicking functions, but these are just the
//! necessary lang items for the compiler. All panics are funneled through this
//! one function. Currently, the actual symbol is declared in the standard
//! library, but the location of this may change over time.

#![allow(dead_code, missing_docs)]
#![unstable(feature = "core_panic",
            reason = "internal details of the implementation of the `panic!` \
                      and related macros",
            issue = "0")]

use fmt;

#[cold] #[inline(never)] // this is the slow path, always
#[lang = "panic"]
pub fn panic(expr_file_line_col: &(&str, &'static str, u32, u32)) -> ! {
    // Use Arguments::new_v1 instead of format_args!("{}", expr) to potentially
    // reduce size overhead. The format_args! macro uses str's Display trait to
    // write expr, which calls Formatter::pad, which must accommodate string
    // truncation and padding (even though none is used here). Using
    // Arguments::new_v1 may allow the compiler to omit Formatter::pad from the
    // output binary, saving up to a few kilobytes.
    let (expr, file, line, col) = *expr_file_line_col;
    panic_fmt(fmt::Arguments::new_v1(&[expr], &[]), &(file, line, col))
}

#[cold] #[inline(never)]
#[lang = "panic_bounds_check"]
fn panic_bounds_check(file_line_col: &(&'static str, u32, u32),
                     index: usize, len: usize) -> ! {
    panic_fmt(format_args!("index out of bounds: the len is {} but the index is {}",
                           len, index), file_line_col)
}

#[cold] #[inline(never)]
pub fn panic_fmt(fmt: fmt::Arguments, file_line_col: &(&'static str, u32, u32)) -> ! {
    #[allow(improper_ctypes)]
    extern {
        #[lang = "panic_fmt"]
        #[unwind]
        fn panic_impl(fmt: fmt::Arguments, file: &'static str, line: u32, col: u32) -> !;
    }
    let (file, line, col) = *file_line_col;
    unsafe { panic_impl(fmt, file, line, col) }
}

/// A struct containing information about the location of a panic.
///
/// This structure is created by the [`location`] method of [`PanicInfo`].
///
/// [`location`]: ../../std/panic/struct.PanicInfo.html#method.location
/// [`PanicInfo`]: ../../std/panic/struct.PanicInfo.html
///
/// # Examples
///
/// ```should_panic
/// use std::panic;
///
/// panic::set_hook(Box::new(|panic_info| {
///     if let Some(location) = panic_info.location() {
///         println!("panic occured in file '{}' at line {}", location.file(), location.line());
///     } else {
///         println!("panic occured but can't get location information...");
///     }
/// }));
///
/// panic!("Normal panic");
/// ```
#[cfg_attr(not(stage0), lang = "location")]
#[derive(Clone, Debug)]
#[stable(feature = "panic_hooks", since = "1.10.0")]
pub struct Location<'a> {
    // Note: If you change the content of order of this structure, please change
    // the following two places to match:
    //  * `location_rvalue()` at ``
    // please change `src/librustc_mir/transform/implicit_location.rs` as well.
    file: &'a str,
    line: u32,
    col: u32,
}

impl<'a> Location<'a> {
    /// Returns the name of the source file from which the panic originated.
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// use std::panic;
    ///
    /// panic::set_hook(Box::new(|panic_info| {
    ///     if let Some(location) = panic_info.location() {
    ///         println!("panic occured in file '{}'", location.file());
    ///     } else {
    ///         println!("panic occured but can't get location information...");
    ///     }
    /// }));
    ///
    /// panic!("Normal panic");
    /// ```
    #[stable(feature = "panic_hooks", since = "1.10.0")]
    pub fn file(&self) -> &'a str {
        self.file
    }

    /// Returns the line number from which the panic originated.
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// use std::panic;
    ///
    /// panic::set_hook(Box::new(|panic_info| {
    ///     if let Some(location) = panic_info.location() {
    ///         println!("panic occured at line {}", location.line());
    ///     } else {
    ///         println!("panic occured but can't get location information...");
    ///     }
    /// }));
    ///
    /// panic!("Normal panic");
    /// ```
    #[stable(feature = "panic_hooks", since = "1.10.0")]
    pub fn line(&self) -> u32 {
        self.line
    }

    /// Returns the column from which the panic originated.
    ///
    /// # Examples
    ///
    /// ```should_panic
    /// #![feature(panic_col)]
    /// use std::panic;
    ///
    /// panic::set_hook(Box::new(|panic_info| {
    ///     if let Some(location) = panic_info.location() {
    ///         println!("panic occured at column {}", location.column());
    ///     } else {
    ///         println!("panic occured but can't get location information...");
    ///     }
    /// }));
    ///
    /// panic!("Normal panic");
    /// ```
    #[unstable(feature = "panic_col", reason = "recently added", issue = "42939")]
    pub fn column(&self) -> u32 {
        self.col
    }

    /// Creates a new location at the given file, line and column.
    #[unstable(feature = "panic_new", reason = "recently added", issue = "99999")]
    pub fn new(file: &'a str, line: u32, col: u32) -> Location<'a> {
        Location { file, line, col }
    }

    #[cfg(stage0)]
    #[unstable(feature = "caller_location", reason = "recently added", issue = "99999")]
    pub fn caller() -> Location<'static> {
        Location {
            file: file!(),
            line: line!(),
            col: column!(),
        }
    }

    /// Obtains the caller's source location.
    ///
    /// User is able to configure the detail of the source location via the
    /// unstable `-Z location-details` rustc option. It is possible that the
    /// returned location is all zero. Therefore, the location is mostly only
    /// suitable for logging.
    #[cfg(not(stage0))]
    #[unstable(feature = "caller_location", reason = "recently added", issue = "99999")]
    #[implicit_caller_location]
    pub fn caller() -> Location<'static> {
        unsafe {
            ::intrinsics::caller_location()
        }
    }
}
