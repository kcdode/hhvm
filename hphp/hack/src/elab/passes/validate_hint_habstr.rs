// Copyright (c) Meta Platforms, Inc. and affiliates.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the "hack" directory of this source tree.

use std::ops::ControlFlow;

use bitflags::bitflags;
use hash::HashMap;
use naming_special_names_rust as sn;
use oxidized::aast_defs::Class_;
use oxidized::aast_defs::Fun_;
use oxidized::aast_defs::Hint;
use oxidized::aast_defs::Hint_;
use oxidized::aast_defs::Method_;
use oxidized::aast_defs::Pos;
use oxidized::aast_defs::Tparam;
use oxidized::aast_defs::Typedef;
use oxidized::aast_defs::WhereConstraintHint;
use oxidized::naming_error::NamingError;
use oxidized::naming_error::UnsupportedFeature;
use oxidized::naming_phase_error::NamingPhaseError;
use oxidized::tast::ReifyKind;

use crate::config::Config;
use crate::Pass;

#[derive(Copy, Clone)]
enum TparamKind {
    Concrete,
    Higher,
}
#[derive(Clone)]
#[derive(Default)]
pub struct ValidateHintHabstrPass {
    tparam_info: HashMap<String, (Pos, bool, TparamKind)>,
    flags: Flags,
}

bitflags! {
    #[derive(Default)]
    struct Flags: u8 {
        const IN_METHOD_OR_FUN = 1 << 0;
        const IN_WHERE_CONSTRAINT = 1 << 1;
    }
}

impl ValidateHintHabstrPass {
    fn in_method_or_fun(&self) -> bool {
        self.flags.contains(Flags::IN_METHOD_OR_FUN)
    }

    fn set_in_method_or_fun(&mut self, value: bool) {
        self.flags.set(Flags::IN_METHOD_OR_FUN, value)
    }

    fn in_where_constraint(&self) -> bool {
        self.flags.contains(Flags::IN_WHERE_CONSTRAINT)
    }

    fn set_in_where_constraint(&mut self, value: bool) {
        self.flags.set(Flags::IN_WHERE_CONSTRAINT, value)
    }

    fn clear_tparams(&mut self) {
        self.tparam_info.clear();
    }
    fn check_tparams<Ex, En>(
        &mut self,
        tparams: &[Tparam<Ex, En>],
        nested: bool,
        errs: &mut Vec<NamingPhaseError>,
    ) {
        // Put each tparam in scope and record its kind; raise errors for
        // shadowed tparams in scope and non-shadowing reuse of previously seen
        // params of higher-kinded params
        tparams
            .iter()
            .filter(|tp| tp.name.name() != sn::typehints::WILDCARD)
            .for_each(|tp| {
                match self.tparam_info.get(tp.name.name()) {
                    // Shadows either a tparam either previously bound in the current scope or bound at some outer scope
                    Some((prev_pos, true, _)) => {
                        errs.push(NamingPhaseError::Naming(NamingError::ShadowedTparam {
                            pos: tp.name.pos().clone(),
                            tparam_name: tp.name.name().to_string(),
                            prev_pos: prev_pos.clone(),
                        }))
                    }
                    // Shares a name with a higher kind tparam which is not in scope
                    Some((_, false, _)) => errs.push(NamingPhaseError::Naming(
                        NamingError::TparamNonShadowingReuse {
                            pos: tp.name.pos().clone(),
                            tparam_name: tp.name.name().to_string(),
                        },
                    )),
                    _ => (),
                }
                let kind = if tp.parameters.is_empty() {
                    TparamKind::Concrete
                } else {
                    TparamKind::Higher
                };
                self.tparam_info.insert(
                    tp.name.name().to_string(),
                    (tp.name.pos().clone(), true, kind),
                );
            });

        tparams
            .iter()
            .for_each(|tp| self.check_tparam(tp, nested, errs));

        // if we are checking tparams of a higher-kinded tparams, remove them from scope
        // but remember we have seen them for non-shadow reuse warnings
        if nested {
            tparams
                .iter()
                .filter(|tp| tp.name.name() != sn::typehints::WILDCARD)
                .for_each(|tp| {
                    self.tparam_info
                        .entry(tp.name.name().to_string())
                        .and_modify(|e| e.1 = false);
                });
        }
    }

    fn check_tparam<Ex, En>(
        &mut self,
        tparam: &Tparam<Ex, En>,
        nested: bool,
        errs: &mut Vec<NamingPhaseError>,
    ) {
        let is_hk = !tparam.parameters.is_empty();
        let name = tparam.name.name();
        let pos = tparam.name.pos();
        // -- Errors related to parameter name ---------------------------------

        // Raise an error if the lowercase tparam name is `this`
        if name.to_lowercase() == sn::typehints::THIS {
            errs.push(NamingPhaseError::Naming(NamingError::ThisReserved(
                pos.clone(),
            )))
        }
        // Raise an error for wildcard top-level tparams
        else if name == sn::typehints::WILDCARD && (!nested || is_hk) {
            errs.push(NamingPhaseError::Naming(
                NamingError::WildcardHintDisallowed(pos.clone()),
            ))
        } else if name.is_empty() || !name.starts_with('T') {
            errs.push(NamingPhaseError::Naming(NamingError::StartWithT(
                pos.clone(),
            )))
        }

        // -- Errors related to features that are not supported in combination
        //    with higher kinded types

        if !tparam.constraints.is_empty() {
            if nested {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: true,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtConstraints,
                    },
                ))
            }
            if is_hk {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: false,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtConstraints,
                    },
                ))
            }
        }

        if tparam.reified != ReifyKind::Erased {
            if nested {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: true,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtReification,
                    },
                ))
            }
            if is_hk {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: false,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtReification,
                    },
                ))
            }
        }

        if !tparam.user_attributes.is_empty() {
            if nested {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: true,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtUserAttrs,
                    },
                ))
            }
            if is_hk {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: false,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtUserAttrs,
                    },
                ))
            }
        }

        if !tparam.variance.is_invariant() {
            if nested {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: true,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtVariance,
                    },
                ))
            }
            if is_hk {
                errs.push(NamingPhaseError::Naming(
                    NamingError::HKTUnsupportedFeature {
                        pos: pos.clone(),
                        because_nested: false,
                        var_name: name.to_string(),
                        feature: UnsupportedFeature::FtVariance,
                    },
                ))
            }
        }

        self.check_tparams(&tparam.parameters, true, errs)
    }
}

// TODO[mjt] we're doing quite a bit of work here to support higher-kinded
// types which are pretty bit-rotted. We should make a call on removing
impl Pass for ValidateHintHabstrPass {
    fn on_ty_class__top_down<Ex, En>(
        &mut self,
        elem: &mut Class_<Ex, En>,
        _cfg: &Config,
        errs: &mut Vec<NamingPhaseError>,
    ) -> ControlFlow<(), ()>
    where
        Ex: Default,
    {
        // [Class_]es exist at the top level so there shouldn't be anything
        // in scope but we clear anyway
        self.clear_tparams();

        // Validate class level tparams and bring them into scope
        self.check_tparams(&elem.tparams, false, errs);

        ControlFlow::Continue(())
    }

    fn on_ty_typedef_top_down<Ex, En>(
        &mut self,
        elem: &mut Typedef<Ex, En>,
        _cfg: &Config,
        errs: &mut Vec<NamingPhaseError>,
    ) -> ControlFlow<(), ()>
    where
        Ex: Default,
    {
        // [Typedef]s exist at the top level so there shouldn't be anything
        // in scope but we clear anyway
        self.clear_tparams();
        self.check_tparams(&elem.tparams, false, errs);
        ControlFlow::Continue(())
    }

    fn on_ty_fun__top_down<Ex, En>(
        &mut self,
        elem: &mut Fun_<Ex, En>,
        _cfg: &Config,
        errs: &mut Vec<NamingPhaseError>,
    ) -> ControlFlow<(), ()>
    where
        Ex: Default,
    {
        // [Fun_]s exist at the top level so there shouldn't be anything
        // in scope but we clear anyway
        self.clear_tparams();
        self.check_tparams(&elem.tparams, false, errs);
        // We want to check hints inside where constraints for functions
        // and methods only (i.e. not class level constraints) so we record
        // this in the context
        self.set_in_method_or_fun(true);
        ControlFlow::Continue(())
    }

    fn on_ty_method__top_down<Ex, En>(
        &mut self,
        elem: &mut Method_<Ex, En>,
        _cfg: &Config,
        errs: &mut Vec<NamingPhaseError>,
    ) -> ControlFlow<(), ()>
    where
        Ex: Default,
    {
        // Validate method level tparams given the already in-scope
        // class level tparams
        self.check_tparams(&elem.tparams, false, errs);
        // We want to check hints inside where constraints for functions
        // and methods only (i.e. not class level constraints) so we record
        // this in the context
        self.set_in_method_or_fun(true);
        ControlFlow::Continue(())
    }

    fn on_ty_where_constraint_hint_top_down(
        &mut self,
        _elem: &mut WhereConstraintHint,
        _cfg: &Config,
        _errs: &mut Vec<NamingPhaseError>,
    ) -> ControlFlow<(), ()> {
        // We want to check hints inside function / method where constraints
        // so we need to record this in the context
        self.set_in_where_constraint(true);
        ControlFlow::Continue(())
    }

    fn on_ty_hint_top_down(
        &mut self,
        elem: &mut Hint,
        _cfg: &Config,
        errs: &mut Vec<NamingPhaseError>,
    ) -> ControlFlow<(), ()> {
        // NB this relies on [Happly] -> [Habstr] elaboration happening
        // in a preceeding top-down pass
        if self.in_method_or_fun() && self.in_where_constraint() {
            if let Hint(pos, box Hint_::Habstr(t, _)) = &elem {
                if let Some((_, true, TparamKind::Higher)) = self.tparam_info.get(t) {
                    errs.push(NamingPhaseError::Naming(
                        NamingError::HKTUnsupportedFeature {
                            pos: pos.clone(),
                            because_nested: false,
                            var_name: t.clone(),
                            feature: UnsupportedFeature::FtWhereConstraints,
                        },
                    ))
                }
            }
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {

    use ocamlrep::rc::RcOc;
    use oxidized::aast_defs::Block;
    use oxidized::aast_defs::FuncBody;
    use oxidized::aast_defs::TypeHint;
    use oxidized::ast_defs::Abstraction;
    use oxidized::ast_defs::ClassishKind;
    use oxidized::ast_defs::Id;
    use oxidized::ast_defs::Variance;
    use oxidized::namespace_env::Env;

    use super::*;
    use crate::transform::Transform;

    fn mk_class(tparams: Vec<Tparam<(), ()>>, methods: Vec<Method_<(), ()>>) -> Class_<(), ()> {
        Class_ {
            span: Default::default(),
            annotation: Default::default(),
            mode: file_info::Mode::Mstrict,
            final_: Default::default(),
            is_xhp: Default::default(),
            has_xhp_keyword: Default::default(),
            kind: ClassishKind::Cclass(Abstraction::Concrete),
            name: Default::default(),
            tparams,
            extends: Default::default(),
            uses: Default::default(),
            xhp_attr_uses: Default::default(),
            xhp_category: Default::default(),
            reqs: Default::default(),
            implements: Default::default(),
            where_constraints: Default::default(),
            consts: Default::default(),
            typeconsts: Default::default(),
            vars: Default::default(),
            methods,
            xhp_children: Default::default(),
            xhp_attrs: Default::default(),
            namespace: RcOc::new(Env::empty(vec![], false, false)),
            user_attributes: Default::default(),
            file_attributes: Default::default(),
            docs_url: Default::default(),
            enum_: Default::default(),
            doc_comment: Default::default(),
            emit_id: Default::default(),
            internal: Default::default(),
            module: Default::default(),
        }
    }

    fn mk_method(
        tparams: Vec<Tparam<(), ()>>,
        where_constraints: Vec<WhereConstraintHint>,
    ) -> Method_<(), ()> {
        Method_ {
            span: Default::default(),
            annotation: Default::default(),
            final_: Default::default(),
            abstract_: Default::default(),
            static_: Default::default(),
            readonly_this: Default::default(),
            visibility: oxidized::tast::Visibility::Public,
            name: Default::default(),
            tparams,
            where_constraints,
            params: Default::default(),
            ctxs: Default::default(),
            unsafe_ctxs: Default::default(),
            body: FuncBody {
                fb_ast: Block(Default::default()),
            },
            fun_kind: oxidized::ast_defs::FunKind::FSync,
            user_attributes: Default::default(),
            readonly_ret: Default::default(),
            ret: TypeHint(Default::default(), None),
            external: Default::default(),
            doc_comment: Default::default(),
        }
    }

    #[test]
    fn test_shadowed_class_member() {
        let cfg = Config::default();
        let mut errs = Vec::default();
        let mut pass = ValidateHintHabstrPass::default();

        let tparam_class: Tparam<(), ()> = Tparam {
            variance: Variance::Invariant,
            name: Id(Default::default(), "T".to_string()),
            parameters: Default::default(),
            constraints: Default::default(),
            reified: ReifyKind::Erased,
            user_attributes: Default::default(),
        };

        let meth = mk_method(vec![tparam_class.clone()], vec![]);

        let mut elem = mk_class(vec![tparam_class], vec![meth]);
        elem.transform(&cfg, &mut errs, &mut pass);

        assert!(matches!(
            errs.as_slice(),
            &[NamingPhaseError::Naming(NamingError::ShadowedTparam { .. })]
        ));
    }

    #[test]
    fn test_shadowed_member() {
        let cfg = Config::default();
        let mut errs = Vec::default();
        let mut pass = ValidateHintHabstrPass::default();

        let tparam: Tparam<(), ()> = Tparam {
            variance: Variance::Invariant,
            name: Id(Default::default(), "T".to_string()),
            parameters: Default::default(),
            constraints: Default::default(),
            reified: ReifyKind::Erased,
            user_attributes: Default::default(),
        };

        let mut elem = mk_method(vec![tparam.clone(), tparam], vec![]);
        elem.transform(&cfg, &mut errs, &mut pass);

        assert!(matches!(
            errs.as_slice(),
            &[NamingPhaseError::Naming(NamingError::ShadowedTparam { .. })]
        ));
    }

    #[test]
    fn test_shadowed_class() {
        let cfg = Config::default();
        let mut errs = Vec::default();
        let mut pass = ValidateHintHabstrPass::default();

        let tparam: Tparam<(), ()> = Tparam {
            variance: Variance::Invariant,
            name: Id(Default::default(), "T".to_string()),
            parameters: Default::default(),
            constraints: Default::default(),
            reified: ReifyKind::Erased,
            user_attributes: Default::default(),
        };

        let mut elem = mk_class(vec![tparam.clone(), tparam], vec![]);
        elem.transform(&cfg, &mut errs, &mut pass);

        assert!(matches!(
            errs.as_slice(),
            &[NamingPhaseError::Naming(NamingError::ShadowedTparam { .. })]
        ));
    }

    #[test]
    fn test_non_shadowed_reuse_class_member() {
        let cfg = Config::default();
        let mut errs = Vec::default();
        let mut pass = ValidateHintHabstrPass::default();

        let tparam_concrete: Tparam<(), ()> = Tparam {
            variance: Variance::Invariant,
            name: Id(Default::default(), "T".to_string()),
            parameters: Default::default(),
            constraints: Default::default(),
            reified: ReifyKind::Erased,
            user_attributes: Default::default(),
        };

        let tparam_higher: Tparam<(), ()> = Tparam {
            variance: Variance::Invariant,
            name: Id(Default::default(), "TH".to_string()),
            parameters: vec![tparam_concrete.clone()],
            constraints: Default::default(),
            reified: ReifyKind::Erased,
            user_attributes: Default::default(),
        };

        let meth = mk_method(vec![tparam_concrete], vec![]);

        let mut elem = mk_class(vec![tparam_higher], vec![meth]);
        elem.transform(&cfg, &mut errs, &mut pass);

        assert!(matches!(
            errs.as_slice(),
            &[NamingPhaseError::Naming(
                NamingError::TparamNonShadowingReuse { .. }
            )]
        ));
    }

    #[test]
    fn test_starts_with_t() {
        let cfg = Config::default();
        let mut errs = Vec::default();
        let mut pass = ValidateHintHabstrPass::default();

        let tparam: Tparam<(), ()> = Tparam {
            variance: Variance::Invariant,
            name: Id(Default::default(), "X".to_string()),
            parameters: Default::default(),
            constraints: Default::default(),
            reified: ReifyKind::Erased,
            user_attributes: Default::default(),
        };

        let mut elem = mk_method(vec![tparam], vec![]);
        elem.transform(&cfg, &mut errs, &mut pass);

        assert!(matches!(
            errs.as_slice(),
            &[NamingPhaseError::Naming(NamingError::StartWithT(..))]
        ));
    }

    #[test]
    fn test_this_reserved() {
        let cfg = Config::default();
        let mut errs = Vec::default();
        let mut pass = ValidateHintHabstrPass::default();

        let tparam: Tparam<(), ()> = Tparam {
            variance: Variance::Invariant,
            name: Id(Default::default(), "This".to_string()),
            parameters: Default::default(),
            constraints: Default::default(),
            reified: ReifyKind::Erased,
            user_attributes: Default::default(),
        };

        let mut elem = mk_method(vec![tparam], vec![]);
        elem.transform(&cfg, &mut errs, &mut pass);

        assert!(matches!(
            errs.as_slice(),
            &[NamingPhaseError::Naming(NamingError::ThisReserved(..))]
        ));
    }
}
