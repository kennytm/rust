use hir::def_id::DefId;
use ty::{TyCtxt, TyAdt};
use mir::*;
use middle::const_val::{ConstInt, ConstVal};
use session::config::LocationDetail;

use rustc_data_structures::indexed_vec::Idx;
use syntax::attr;
use syntax::ast::NodeId;
use syntax::symbol::Symbol;
use syntax::abi::Abi;
use syntax::codemap::original_sp;
use syntax_pos::{Span, DUMMY_SP};

use std::mem;

/// Whether the function has the `#[rustc_implicit_caller_location]` attribute.
pub fn is_implicit_caller_location_fn(tcx: TyCtxt, node_id: NodeId) -> bool {
    attr::contains_name(tcx.hir.attrs(node_id), "rustc_implicit_caller_location")
}

/// Whether the parent of the closure has the `#[rustc_implicit_caller_location]` attribute.
pub fn is_implicit_caller_location_closure(tcx: TyCtxt, node_id: NodeId) -> bool {
    let parent_node_id = tcx.hir.get_parent(node_id);
    is_implicit_caller_location_fn(tcx, parent_node_id)
}

/// Whether the called function is `caller_location()`.
pub fn is_caller_location_intrinsic(tcx: TyCtxt, def_id: DefId) -> bool {
    tcx.fn_sig(def_id).abi() == Abi::RustIntrinsic && tcx.item_name(def_id) == "caller_location"
}

/// Obtains the location tuple corresponding to the given `Span`.
pub fn location_tuple(tcx: TyCtxt, span: Span) -> (Symbol, u32, u32) {
    let span = original_sp(span, DUMMY_SP);
    let location_detail = tcx.sess.opts.debugging_opts.location_detail;
    let loc = tcx.sess.codemap().lookup_char_pos(span.lo());

    let file = if location_detail.contains(LocationDetail::FILE) {
        Symbol::intern(&loc.file.name)
    } else {
        Symbol::intern("<redacted>")
    };
    let line = if location_detail.contains(LocationDetail::LINE) {
        loc.line as u32
    } else {
        0
    };
    let column = if location_detail.contains(LocationDetail::COLUMN) {
        loc.col.0 as u32
    } else {
        0
    };

    (file, line, column)
}

/// Obtains the `core::panicking::Location` rvalue corresponding to the given `Span`.
pub fn location_rvalue<'a, 'tcx>(tcx: TyCtxt<'a, 'tcx, 'tcx>, span: Span) -> Rvalue<'tcx> {
    let (file, line, column) = location_tuple(tcx, span);
    let fields = vec![
        Operand::Constant(box Constant {
            span,
            ty: tcx.mk_static_str(),
            literal: Literal::Value { value: ConstVal::Str(file.as_str()) },
        }),
        Operand::Constant(box Constant {
            span,
            ty: tcx.types.u32,
            literal: Literal::Value { value: ConstVal::Integral(ConstInt::U32(line)) },
        }),
        Operand::Constant(box Constant {
            span,
            ty: tcx.types.u32,
            literal: Literal::Value { value: ConstVal::Integral(ConstInt::U32(column)) },
        }),
    ];

    let location_ty = tcx.mk_location_ty();
    let (adt, substs) = match location_ty.sty {
        TyAdt(adt, substs) => (adt, substs),
        _ => bug!("`location` lang-item is not a structure: {:?}", location_ty),
    };

    Rvalue::Aggregate(box AggregateKind::Adt(adt, 0, substs, None), fields)
}

pub fn replace_caller_location<'tcx>(
    tcx: TyCtxt,
    data: &mut BasicBlockData<'tcx>,
    rvalue: Rvalue<'tcx>,
) {
    let lvalue;
    let source_info;
    {
        let terminator = data.terminator_mut();
        source_info = terminator.source_info;
        let target;
        match terminator.kind {
            TerminatorKind::Call {
                func: Operand::Constant(ref func),
                destination: Some(ref mut destination),
                ..
            } => {
                let def_id = match func.literal {
                    Literal::Item { def_id, .. } => def_id,
                    Literal::Value { value: ConstVal::Function(def_id, _) } => def_id,
                    _ => return,
                };
                if !is_caller_location_intrinsic(tcx, def_id) {
                    return;
                }
                lvalue = mem::replace(&mut destination.0, Lvalue::Local(Local::new(0)));
                target = destination.1;
            }
            _ => return,
        }
        terminator.kind = TerminatorKind::Goto { target };
    };

    data.statements.push(Statement {
        source_info,
        kind: StatementKind::Assign(lvalue, rvalue)
    });
}
