// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use rustc::hir::def_id::DefId;
use rustc::ty::TyCtxt;
use rustc::mir::{Mir, Constant, Literal, Location};
use rustc::mir::visit::Visitor;
use rustc::mir::transform::{MirPass, MirSource};

use rustc_data_structures::array_vec::ArrayVec;

use syntax::attr::{InlineAttr, find_inline_attr};
use syntax::errors::Handler;

struct Checker<'a> {
    diagnostic: &'a Handler,
    bad_lang_items: ArrayVec<[DefId; 3]>,
}

pub struct CallerLocationCheck;

impl MirPass for CallerLocationCheck {
    fn run_pass<'a, 'tcx>(
        &self,
        tcx: TyCtxt<'a, 'tcx, 'tcx>,
        source: MirSource,
        mir: &mut Mir<'tcx>,
    ) {
        match source {
            MirSource::Fn(item_id) => {
                if find_inline_attr(None, tcx.hir.attrs(item_id)) == InlineAttr::Semantic {
                    return;
                }
            },
            MirSource::Promoted(..) => return,
            _ => {},
        }

        let mut bad_lang_items = ArrayVec::new();
        bad_lang_items.extend([
            tcx.lang_items.caller_file(),
            tcx.lang_items.caller_line(),
            tcx.lang_items.caller_column(),
        ].iter().filter_map(|a| *a));

        Checker {
            diagnostic: tcx.sess.diagnostic(),
            bad_lang_items,
        }.visit_mir(mir);
    }
}

impl<'a, 'tcx> Visitor<'tcx> for Checker<'a> {
    fn visit_mir(&mut self, mir: &Mir<'tcx>) {
        for promoted in &mir.promoted {
            self.visit_mir(promoted);
        }
        self.super_mir(mir);
    }

    fn visit_constant(&mut self, constant: &Constant<'tcx>, location: Location) {
        if let Literal::Item { def_id, .. } = constant.literal {
            if self.bad_lang_items.contains(&def_id) {
                self.diagnostic.span_err(
                    constant.span,
                    "Cannot read caller location outside of `#[inline(semantic)]` function",
                );
            }
        }
        self.super_constant(constant, location);
    }
}