#![warn(warnings)]

use syntax::ext::base::{AttrProcMacro, ExtCtxt};
use syntax::ext::build::AstBuilder;
use syntax::tokenstream::TokenStream;
use syntax::parse::parser::Parser;
use syntax::parse::token::{Token, Nonterminal, LazyTokenStream};
use syntax::parse::PResult;
use syntax::ast::*;
use syntax::ptr::P;
use syntax_pos::Span;
use syntax::codemap::{ExpnInfo, ExpnFormat, NameAndSpan};
use errors::DiagnosticBuilder;

use deriving::call_intrinsic;

use std::rc::Rc;
use std::mem;

pub struct Expand;

impl AttrProcMacro for Expand {
    fn expand<'cx>(
        &self,
        ecx: &'cx mut ExtCtxt,
        attr_span: Span,
        _annotation: TokenStream,
        annotated: TokenStream,
    ) -> TokenStream {
        let trees = annotated.into_trees().collect::<Vec<_>>();
        let parser = ecx.new_parser_from_tts(&trees);
        let mut transformer = Transformer { ecx, parser, attr_span };
        match transformer.transform() {
            Ok(token) => token.into(),
            Err(mut diag) => {
                diag.emit();
                TokenStream::empty()
            }
        }
    }
}

struct Transformer<'cx, 'a: 'cx> {
    ecx: &'cx ExtCtxt<'a>,
    parser: Parser<'a>,
    attr_span: Span,
}

impl<'cx, 'a> Transformer<'cx, 'a> {
    fn wrap_block(&self, span: Span, block: &mut P<Block>) {
        let ecx = self.ecx;
        let orig_block = mem::replace(block, ecx.block(span, Vec::new()));

        // let __closure = |__location| { ... };
        let closure = ecx.lambda1(
            span,
            ecx.expr_block(orig_block),
            ecx.ident_of("__location"), // FIXME: hygiene?
        ).map(|mut expr| {
            if let ExprKind::Closure(ref mut capture_by, _, _, _) = expr.node {
                *capture_by = CaptureBy::Value;
            }
            expr
        });

        // std::ops::FnOnce::call_once(__closure, (std::intrinsics::caller_location(),))
        let span = self.allow_internal_unstable(span);
        let call = ecx.expr_call_global(
            span,
            ecx.std_path(&["ops", "FnOnce", "call_once"]),
            vec![
                closure,
                ecx.expr_tuple(span, vec![
                    call_intrinsic(ecx, span, "caller_location", Vec::new())
                ]),
            ],
        );

        *block = ecx.block_expr(call)
    }

    fn error(&self, span: Span) -> DiagnosticBuilder<'a> {
        self.parser.diagnostic().mut_span_err(
            span,
            "#[implicit_caller_location] can only be applied on functions",
        )
    }

    fn allow_internal_unstable(&self, span: Span) -> Span {
        self.ecx.current_expansion.mark.set_expn_info(ExpnInfo {
            call_site: span,
            callee: NameAndSpan {
                format: ExpnFormat::MacroAttribute(self.ecx.name_of("implicit_caller_location")),
                span: None,
                allow_internal_unstable: true,
                allow_internal_unsafe: false,
            },
        });
        span.with_ctxt(self.ecx.backtrace())
    }

    fn make_attributes(&self) -> Vec<Attribute> {
        let ecx = self.ecx;
        let span = self.allow_internal_unstable(self.attr_span);
        let word = ecx.name_of("rustc_implicit_caller_location");
        vec![
            ecx.attribute(span, ecx.meta_word(span, word)),
            ecx.attribute(span, ecx.meta_word(span, ecx.name_of("inline"))),
            //^ #[inline] is needed to expose the MIR for MIR inlining
        ]
    }

    fn transform(&mut self) -> PResult<'a, Token> {
        let nt = if let Some(item) = self.transform_item()? {
            (Nonterminal::NtItem(item), LazyTokenStream::new())
        } else {
            match self.transform_impl_item() {
                Ok(impl_item) => (Nonterminal::NtImplItem(impl_item), LazyTokenStream::new()),
                Err(mut diag) => {
                    self.parser.diagnostic().cancel(&mut diag);
                    let trait_item = self.transform_trait_item()?;
                    (Nonterminal::NtTraitItem(trait_item), LazyTokenStream::new())
                }
            }
        };
        Ok(Token::Interpolated(Rc::new(nt)))
    }

    fn transform_item(&mut self) -> PResult<'a, Option<P<Item>>> {
        if let Some(item) = self.parser.parse_item()? {
            item.and_then(|mut item| {
                let span = item.span;
                if let ItemKind::Fn(_, _, _, _, _, ref mut block) = item.node {
                    self.wrap_block(span, block);
                } else {
                    return Err(self.error(span));
                }

                item.attrs.append(&mut self.make_attributes());
                Ok(Some(P(item)))
            })
        } else {
            Ok(None)
        }
    }

    fn transform_impl_item(&mut self) -> PResult<'a, ImplItem> {
        let mut at_end = false;
        let mut impl_item = self.parser.parse_impl_item(&mut at_end)?;
        let span = impl_item.span;
        if let ImplItemKind::Method(_, ref mut block) = impl_item.node {
            self.wrap_block(span, block);
        } else {
            return Err(self.error(span));
        }
        impl_item.attrs.append(&mut self.make_attributes());
        Ok(impl_item)
    }

    fn transform_trait_item(&mut self) -> PResult<'a, TraitItem> {
        let mut at_end = false;
        let mut trait_item = self.parser.parse_trait_item(&mut at_end)?;
        let span = trait_item.span;
        if let TraitItemKind::Method(_, Some(ref mut block)) = trait_item.node {
            self.wrap_block(span, block);
        } else {
            return Err(self.error(span));
        }
        trait_item.attrs.append(&mut self.make_attributes());
        Ok(trait_item)
    }
}
