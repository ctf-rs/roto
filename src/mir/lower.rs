mod match_expr;

use std::{any::TypeId, collections::HashMap};

use inetnum::addr::Prefix;

use super::{
    Block, Instruction, Item, Mir, Place, Projection, Value, Var, VarKind, ty,
};

use crate::{
    ast::{self, CompoundAssignOp, Expr, Identifier, IntType, Literal},
    ice,
    ir_printer::{IrPrinter, Printable},
    label::{LabelRef, LabelStore},
    mir::{ItemKind, Ty, TyRef},
    module::ModuleTree,
    parser::meta::{Meta, MetaId},
    runtime::{Rt, RuntimeFunctionRef},
    typechecker::{
        self, PathValue, ResolvedPath,
        info::TypeInfo,
        scope::{DeclarationKind, ResolvedName, ScopeRef, ValueKind},
        types::{
            EarlyReturn, FunctionDefinition, Intrinsic, Signature, Type,
        },
    },
    value::ErasedList,
};

pub struct Lowerer<'r> {
    function_scope: ScopeRef,
    tmp_idx: usize,
    blocks: Vec<Block>,
    runtime: &'r Rt,
    type_info: &'r mut TypeInfo,
    label_store: &'r mut LabelStore,
    return_type: TyRef,

    /// All the stack slots that are allocated in each scope
    ///
    /// The first element contains the variables allocated in the function body,
    /// the last element is our current scope and between are the parent
    /// scopes. At the end of a block we drop all variables in the last element
    /// and then pop that element.
    stack_slots: Vec<Vec<(Var, TyRef)>>,
    vars: Vec<(Var, TyRef)>,
}

pub fn lower_to_mir(
    tree: &ModuleTree,
    runtime: &Rt,
    type_info: &mut TypeInfo,
    label_store: &mut LabelStore,
    order: &[ResolvedName],
) -> Mir {
    let mut mir = Lowerer::tree(runtime, type_info, tree, label_store, order);
    mir.eliminate_dead_code();
    mir
}

impl<'r> Lowerer<'r> {
    fn new(
        runtime: &'r Rt,
        type_info: &'r mut TypeInfo,
        function_name: &Meta<Identifier>,
        label_store: &'r mut LabelStore,
        is_constant_definition: bool,
    ) -> Self {
        let function_scope = type_info.function_scope(function_name);

        let return_type = if is_constant_definition {
            type_info.type_of(function_name)
        } else {
            let signature = type_info.function_signature(function_name);
            signature.return_type
        };

        let return_type = type_info.convert(&return_type);

        Self {
            tmp_idx: 0,
            type_info,
            runtime,
            return_type,
            function_scope,
            blocks: Vec::new(),
            stack_slots: Vec::new(),
            vars: Vec::new(),
            label_store,
        }
    }

    /// Add a new block to the blocks in the program.
    fn new_block(&mut self, label: LabelRef) {
        self.blocks.push(Block {
            label,
            instructions: Vec::new(),
        })
    }

    /// Get the label of the block we are currently building
    fn current_label(&self) -> LabelRef {
        self.blocks.last().unwrap().label
    }

    /// Create a new unique temporary variable that won't be dropped
    fn undropped_tmp(&mut self) -> Var {
        let var = Var {
            scope: self.function_scope,
            kind: VarKind::Tmp(self.tmp_idx),
        };
        self.tmp_idx += 1;
        var
    }

    /// Create a new temporary variable that will be dropped
    fn tmp(&mut self, ty: TyRef) -> Var {
        let var = Var {
            scope: self.function_scope,
            kind: VarKind::Tmp(self.tmp_idx),
        };
        self.tmp_idx += 1;
        self.add_live_variable(var.clone(), ty);
        var
    }

    fn assign_to_var(&mut self, value: Value, ty: TyRef) -> Var {
        if let Value::Move(x) = value {
            return x;
        }
        let to = self.tmp(ty);
        self.do_assign(Place::new(to.clone(), ty), ty, value);
        to
    }

    /// Lower a syntax tree
    fn tree(
        runtime: &Rt,
        type_info: &mut TypeInfo,
        tree: &ModuleTree,
        label_store: &mut LabelStore,
        order: &[ResolvedName],
    ) -> Mir {
        let mut items = HashMap::new();

        for m in &tree.modules {
            for d in &m.ast.declarations {
                match d {
                    ast::Declaration::FilterMap(x) => {
                        items.insert(
                            type_info.resolved_name(&x.ident),
                            Lowerer::new(
                                runtime,
                                type_info,
                                &x.ident,
                                label_store,
                                false,
                            )
                            .filter_map(x),
                        );
                    }
                    ast::Declaration::Function(x) => {
                        items.insert(
                            type_info.resolved_name(&x.ident),
                            Lowerer::new(
                                runtime,
                                type_info,
                                &x.ident,
                                label_store,
                                false,
                            )
                            .function(x),
                        );
                    }
                    // We give tests special names, so that they can't be referenced from Roto.
                    // It's a bit of a hack, but works well enough.
                    ast::Declaration::Test(x) => {
                        items.insert(
                            type_info.resolved_name(&x.ident),
                            Lowerer::new(
                                runtime,
                                type_info,
                                &Meta {
                                    node: format!("test#{}", x.ident).into(),
                                    id: x.ident.id,
                                },
                                label_store,
                                false,
                            )
                            .test(x),
                        );
                    }
                    ast::Declaration::Const(x) => {
                        items.insert(
                            type_info.resolved_name(&x.ident),
                            Lowerer::new(
                                runtime,
                                type_info,
                                &Meta {
                                    node: format!("constant#{}", x.ident)
                                        .into(),
                                    id: x.ident.id,
                                },
                                label_store,
                                true,
                            )
                            .constant(x),
                        );
                    }
                    // Ignore the rest
                    _ => {}
                }
            }
        }

        let items = order.iter().flat_map(|n| items.remove(n)).collect();
        Mir { items }
    }

    /// Lower a filtermap
    fn filter_map(self, fm: &ast::FilterMap) -> Item {
        let ast::FilterMap {
            ident,
            body,
            params,
            ..
        } = fm;

        let signature = self.type_info.function_signature(ident);
        self.function_like(ident, params, &signature.return_type, body)
    }

    fn function(self, function: &ast::FunctionDeclaration) -> Item {
        let name = self.type_info.resolved_name(&function.ident);
        let dec = self.type_info.scope_graph.get_declaration(name);

        let DeclarationKind::Function(Some(func_dec)) = dec.kind else {
            ice!();
        };

        let ret = &func_dec.signature.return_type;

        self.function_like(
            &function.ident,
            &function.params,
            ret,
            &function.body,
        )
    }

    fn test(self, test: &ast::Test) -> Item {
        let ident = Meta {
            node: format!("test#{}", *test.ident).into(),
            id: test.ident.id,
        };
        let return_type = Type::verdict(Type::unit(), Type::unit());
        let params = ast::Params(Vec::new());
        self.function_like(&ident, &params, &return_type, &test.body)
    }

    fn constant(mut self, constant: &ast::ConstantDeclaration) -> Item {
        let scope = self.type_info.function_scope(&constant.ident);
        let label = self.label_store.new_label("$entry".into());
        self.new_block(label);

        self.stack_slots.push(Vec::new());
        self.stack_slots.push(Vec::new());

        let last = self.expr(&constant.expr);

        let ty = self.type_info.type_of(&constant.ident);
        let mir_ty = self.type_info.convert(&ty);

        let tmp = self.assign_to_var(last, mir_ty);

        self.remove_live_variable(&tmp);

        let to_drop = self.stack_slots.pop().unwrap();
        for (var, ty) in to_drop.into_iter().rev() {
            self.emit_drop(Place::new(var, ty), ty);
        }

        self.emit_return(tmp);

        let resolved_name = self.type_info.resolved_name(&constant.ident);
        let name = self.type_info.full_name(&resolved_name);

        Item {
            name,
            scope,
            variables: self.vars,
            ty: ItemKind::Constant {
                ty,
                mir_ty,
                name: resolved_name,
            },
            tmp_idx: self.tmp_idx,
            blocks: self.blocks,
        }
    }

    /// Lower a function-like construct (i.e. a function, filtermap or test)
    fn function_like(
        mut self,
        ident: &Meta<Identifier>,
        params: &ast::Params,
        return_type: &Type,
        body: &Meta<ast::Block>,
    ) -> Item {
        let scope = self.type_info.function_scope(ident);
        let label = self.label_store.new_label("$entry".into());
        self.new_block(label);

        let mut parameter_types = Vec::new();

        for (x, _) in &params.0 {
            let ty = self.type_info.type_of(x);
            parameter_types.push((self.type_info.resolved_name(x), ty));
        }

        self.stack_slots.push(Vec::new());

        let mut mir_parameter_types = Vec::new();
        let mut parameters = Vec::new();
        for (name, ty) in &parameter_types {
            let var = Var {
                scope: name.scope,
                kind: VarKind::Explicit(name.ident),
            };
            parameters.push(var.clone());
            let ty = self.type_info.convert(ty);
            mir_parameter_types.push(ty);
            self.add_live_variable(var, ty);
        }

        let signature = Signature {
            types: Vec::new(),
            parameter_types: parameter_types
                .iter()
                .map(|x| &x.1)
                .cloned()
                .collect(),
            return_type: return_type.clone(),
        };

        let last = self.block(body);
        let return_type = self.type_info.convert(return_type);
        let tmp = self.assign_to_var(last, return_type);

        let mir_signature = ty::Signature {
            parameter_types: mir_parameter_types,
            return_type,
        };

        let to_drop = self.stack_slots.pop().unwrap();
        for (var, ty) in to_drop.into_iter().rev() {
            self.emit_drop(Place::new(var, ty), ty);
        }

        self.emit_return(tmp);

        let name = self.type_info.resolved_name(ident);
        let name = self.type_info.full_name(&name);

        Item {
            name,
            scope,
            variables: self.vars,
            ty: ItemKind::Function {
                parameters,
                signature,
                mir_signature,
            },
            tmp_idx: self.tmp_idx,
            blocks: self.blocks,
        }
    }

    fn block_expr(&mut self, block: &Meta<ast::Block>) -> Value {
        let val = self.block(block);

        let ty = self.type_info.type_of(block);
        let ty = self.type_info.convert(&ty);

        let res = self.undropped_tmp();
        self.emit_assign(Place::new(res.clone(), ty), ty, val);

        self.add_live_variable(res.clone(), ty);
        Value::Move(res)
    }

    fn block(&mut self, block: &Meta<ast::Block>) -> Value {
        self.stack_slots.push(Vec::new());

        // Resulting operand is ignored
        for stmt in &block.stmts {
            self.stmt(stmt);
        }

        let op = match &block.last {
            Some(expr) => self.expr(expr),
            None => Value::Const(ast::Literal::Unit, TyRef::UNIT),
        };

        let ty = self.type_info.type_of(block);
        let ty = self.type_info.convert(&ty);
        let final_var = self.assign_to_var(op.clone(), ty);
        self.remove_live_variable(&final_var);

        let to_drop = self.stack_slots.pop().unwrap();

        // If the block diverges, which happens for instance with an explicit
        // return, then we don't need to drop anything. That will only generate
        // noise in the MIR.
        if !self.type_info.diverges(block) {
            // Drop order is reversed
            for (var, ty) in to_drop.into_iter().rev() {
                self.emit_drop(Place::new(var, ty), ty);
            }
        }

        Value::Move(final_var)
    }

    fn drop_var(&mut self, var: Var) {
        let ty = self.remove_live_variable(&var);
        self.emit_drop(Place::new(var, ty), ty);
    }

    fn stmt(&mut self, stmt: &Meta<ast::Stmt>) {
        match &**stmt {
            ast::Stmt::Let(ident, _, expr) => {
                let val = self.expr(expr);
                let name = self.type_info.resolved_name(ident);
                let ty = self.type_info.type_of(ident);
                let ty = self.type_info.convert(&ty);

                let to = Var {
                    scope: name.scope,
                    kind: VarKind::Explicit(**ident),
                };

                self.add_live_variable(to.clone(), ty);
                self.do_assign(Place::new(to, ty), ty, val);
            }
            ast::Stmt::Expr(expr) => {
                let value = self.expr(expr);
                let ty = self.type_info.type_of(expr);
                let ty = self.type_info.convert(&ty);
                let value = self.assign_to_var(value, ty);
                self.drop_var(value);
            }
        }
    }

    fn expr(&mut self, expr: &Meta<ast::Expr>) -> Value {
        let id = expr.id;
        match &**expr {
            ast::Expr::Return(return_kind, expr) => {
                self.r#return(return_kind, expr)
            }
            ast::Expr::Literal(literal) => self.literal(literal),
            ast::Expr::Block(block) => self.block_expr(block),
            ast::Expr::Match(r#match) => self.r#match(id, r#match),
            ast::Expr::FunctionCall(function, arguments) => {
                self.function_call(id, function, arguments)
            }
            ast::Expr::Access(expr, field) => self.access(expr, field),
            ast::Expr::Path(path) => self.path(id, path),
            ast::Expr::Record(record) | ast::Expr::TypedRecord(_, record) => {
                self.record(id, record)
            }
            ast::Expr::List(list) => self.list(id, list),
            ast::Expr::Not(expr) => self.not(expr),
            ast::Expr::Negate(expr) => self.negate(expr),
            ast::Expr::Assign(expr, field) => self.assign(expr, field),
            ast::Expr::CompoundAssign(c) => self.compound_assign(c),
            ast::Expr::BinOp(left, op, right) => self.binop(left, op, right),
            ast::Expr::IfElse(condition, then, r#else) => {
                self.if_else(id, condition, then, r#else)
            }
            ast::Expr::While(condition, block) => {
                self.r#while(condition, block)
            }
            ast::Expr::For(name, expr, body) => self.r#for(name, expr, body),
            ast::Expr::QuestionMark(expr) => self.question_mark(expr),
            ast::Expr::FString(parts) => self.f_string(parts),
        }
    }

    fn r#return(
        &mut self,
        return_kind: &ast::ReturnKind,
        expr: &Option<Box<Meta<ast::Expr>>>,
    ) -> Value {
        let val = match expr {
            Some(expr) => {
                let ty = self.type_info.type_of(&**expr);
                let ty = self.type_info.convert(&ty);
                (self.expr(expr), ty)
            }
            None => {
                (Value::Const(ast::Literal::Unit, TyRef::UNIT), TyRef::UNIT)
            }
        };

        match return_kind {
            ast::ReturnKind::Return => self.return_value(val.0),
            ast::ReturnKind::Accept => {
                let ty = self.return_type;
                let val = self.make_enum(ty, "Accept".into(), &[val]);
                self.return_value(val)
            }
            ast::ReturnKind::Reject => {
                let ty = self.return_type;
                let val = self.make_enum(ty, "Reject".into(), &[val]);
                self.return_value(val)
            }
        }
    }

    fn question_mark(&mut self, expr: &Meta<ast::Expr>) -> Value {
        let current_label = self.current_label();
        let lbl_return_none = self
            .label_store
            .wrap_internal(current_label, Identifier::from("return-none"));
        let continue_lbl = self.label_store.next(current_label);

        let examinee = self.expr(expr);
        let examinee_ty = self.type_info.type_of(expr);
        let examinee_ty = self.type_info.convert(&examinee_ty);
        let examinee = self.assign_to_var(examinee, examinee_ty);

        // `?` works on Option, Result and Verdict alike: all three are
        // structurally the same 2-variant enum, with variant 0 being the
        // success case (exactly one field: Some/Ok/Accept) and variant 1
        // being the failure case (zero or one field: None/Err/Reject).
        // Look up the examinee's actual variant names/fields here instead
        // of hardcoding "Some"/"None", so the same lowering works for all
        // three types.
        let Ty::Enum(variants) = self.type_info.ty_pool.get(examinee_ty)
        else {
            ice!("`?` examinee is not an enum");
        };
        let (success_name, _) = variants[0];
        let (failure_name, failure_fields) = variants[1].clone();
        let failure_field_ty = failure_fields.first().copied();

        let discriminant = self.undropped_tmp();
        self.emit_assign(
            Place::new(discriminant.clone(), TyRef::U8),
            TyRef::U8,
            Value::Discriminant(examinee.clone()),
        );

        // If the failure variant carries a field (Err/Reject), extract it
        // from the examinee before branching away: once we jump to the
        // early-return block we can no longer meaningfully project into
        // the examinee's own variant.
        let failure_arg = failure_field_ty.map(|field_ty| {
            (
                Value::Clone(Place {
                    var: examinee.clone(),
                    root_ty: examinee_ty,
                    projection: vec![Projection::VariantField(
                        failure_name,
                        0,
                    )],
                }),
                field_ty,
            )
        });

        self.emit_switch(
            discriminant,
            vec![(0, continue_lbl)],
            Some(lbl_return_none),
        );

        self.new_block(lbl_return_none);
        let ty = self.return_type;
        let args: Vec<(Value, TyRef)> = failure_arg.into_iter().collect();
        let val = self.make_enum(ty, failure_name, &args);
        let _ = self.return_value(val);

        self.new_block(continue_lbl);
        let ty = self.type_info.type_of(expr);
        let ty = self.type_info.convert(&ty);
        Value::Clone(Place {
            var: examinee,
            root_ty: ty,
            projection: vec![Projection::VariantField(success_name, 0)],
        })
    }

    /// Looks up the variant names of a 2-variant enum type (Option, Result
    /// or Verdict): `(success_variant_name, failure_variant_name)`, i.e.
    /// `(Some, None)`, `(Ok, Err)` or `(Accept, Reject)`.
    fn enum_variant_names(&self, ty: TyRef) -> (Identifier, Identifier) {
        let Ty::Enum(variants) = self.type_info.ty_pool.get(ty) else {
            ice!("expected a 2-variant enum (Option, Result or Verdict)");
        };
        (variants[0].0, variants[1].0)
    }

    /// Emits a two-way branch on the discriminant of `receiver` (an
    /// Option, Result or Verdict value): runs `on_success` when the
    /// discriminant is 0 (Some/Ok/Accept), `on_failure` when it is 1
    /// (None/Err/Reject), and joins the two branches' values (each of type
    /// `result_ty`) back into one.
    fn branch_on_discriminant(
        &mut self,
        receiver: Var,
        result_ty: TyRef,
        on_success: impl FnOnce(&mut Self) -> Value,
        on_failure: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        let current_label = self.current_label();
        let lbl_cont = self.label_store.next(current_label);
        let lbl_success = self.label_store.wrap_internal(
            current_label,
            Identifier::from("intrinsic-success"),
        );
        let lbl_failure = self.label_store.wrap_internal(
            current_label,
            Identifier::from("intrinsic-failure"),
        );

        let discriminant = self.undropped_tmp();
        self.emit_assign(
            Place::new(discriminant.clone(), TyRef::U8),
            TyRef::U8,
            Value::Discriminant(receiver),
        );
        self.emit_switch(
            discriminant,
            vec![(0, lbl_success)],
            Some(lbl_failure),
        );

        self.new_block(lbl_success);
        let op = on_success(self);
        let res = self.undropped_tmp();
        // Use `do_assign`, not a plain `emit_assign`, so that a
        // `Value::Move` result (e.g. from `make_enum`) correctly removes
        // its source variable from live-variable tracking. Otherwise that
        // variable would still look "live" at scope-exit and get dropped
        // a second time there, even though its bytes were already copied
        // into `res` here (a double-drop/use-after-free for any type with
        // drop glue, e.g. RotoString).
        self.do_assign(Place::new(res.clone(), result_ty), result_ty, op);
        self.emit_jump(lbl_cont);

        self.new_block(lbl_failure);
        let op = on_failure(self);
        self.do_assign(Place::new(res.clone(), result_ty), result_ty, op);
        self.emit_jump(lbl_cont);

        self.new_block(lbl_cont);
        self.add_live_variable(res.clone(), result_ty);
        Value::Move(res)
    }

    /// Converts a 2-variant enum value (`receiver`, an Option or Result)
    /// into another structurally-identical one (`target_ty`): the success
    /// variant's field is always carried over 1:1 (e.g. `Some(v)`/`Ok(v)`
    /// -> `Ok(v)`/`Accept(v)`). The failure variant is either constructed
    /// from `failure_override` (used for `ok_or`/`ok_or_reject`, where the
    /// caller supplies the new error/reject value, since `None` carries no
    /// value of its own to reuse) or with no field at all (used for
    /// `Result::ok`, which discards the error).
    fn intrinsic_convert(
        &mut self,
        receiver: Var,
        receiver_ty: TyRef,
        target_ty: TyRef,
        failure_override: Option<(Var, TyRef)>,
    ) -> Value {
        let (success_name, _) = self.enum_variant_names(receiver_ty);
        let (target_success_name, target_failure_name) =
            self.enum_variant_names(target_ty);

        let recv_for_success = receiver.clone();
        // The override (if any) is only ever consumed on the failure
        // path (below). Clone the handle so the success path can drop it
        // explicitly instead: `branch_on_discriminant` unconditionally
        // lowers *both* branches into the function body, so if we don't
        // balance this here, the success path would silently leave this
        // value undropped whenever it's actually taken at runtime, while
        // our own liveness bookkeeping (which only sees the failure
        // path's `Value::Move`) would incorrectly believe it was already
        // handled everywhere.
        let override_for_success = failure_override.clone();
        self.branch_on_discriminant(
            receiver,
            target_ty,
            move |this| {
                if let Some((arg, arg_ty)) = override_for_success {
                    // Emit the actual drop instruction so the success
                    // path is runtime-correct, but do NOT also remove it
                    // from the compiler's liveness bookkeeping here: that
                    // bookkeeping is a single, branch-unaware, compile
                    // time structure, and the failure branch below
                    // already removes it exactly once (via its own
                    // `Value::Move`), regardless of which branch is
                    // actually taken at runtime.
                    this.emit_drop(Place::new(arg, arg_ty), arg_ty);
                }

                let field = Value::Clone(Place {
                    var: recv_for_success,
                    root_ty: receiver_ty,
                    projection: vec![Projection::VariantField(
                        success_name,
                        0,
                    )],
                });
                let field_ty = {
                    let Ty::Enum(variants) =
                        this.type_info.ty_pool.get(target_ty)
                    else {
                        ice!();
                    };
                    variants[0].1[0]
                };
                this.make_enum(
                    target_ty,
                    target_success_name,
                    &[(field, field_ty)],
                )
            },
            move |this| match failure_override {
                Some((arg, _)) => {
                    let field_ty = {
                        let Ty::Enum(variants) =
                            this.type_info.ty_pool.get(target_ty)
                        else {
                            ice!();
                        };
                        variants[1].1[0]
                    };
                    this.make_enum(
                        target_ty,
                        target_failure_name,
                        &[(Value::Move(arg), field_ty)],
                    )
                }
                None => this.make_enum(target_ty, target_failure_name, &[]),
            },
        )
    }

    /// Converts an Option or Result (`receiver`) into a Verdict
    /// (`target_ty`), always constructing the Accept variant: on
    /// Some/Ok, carrying over the receiver's own field; on None/Err,
    /// using `default` instead and discarding whatever the receiver's
    /// failure variant held (if anything). This is a "fail open"
    /// conversion for `ok_or_accept`, for firewalls that would rather
    /// accept-with-a-default than reject when a value couldn't be
    /// parsed/found.
    /// Builds an `Option` (`target_ty`) out of one variant of a 2-variant
    /// enum receiver: `Result::ok`/`Verdict::accepted` take the success
    /// variant's field (`from_success`), `Result::err`/`Verdict::rejected`
    /// take the failure variant's. The other variant becomes `None`.
    fn intrinsic_to_option(
        &mut self,
        receiver: Var,
        receiver_ty: TyRef,
        target_ty: TyRef,
        from_success: bool,
    ) -> Value {
        let (success_name, failure_name) =
            self.enum_variant_names(receiver_ty);
        let (some_name, none_name) = self.enum_variant_names(target_ty);
        let wanted = if from_success {
            success_name
        } else {
            failure_name
        };

        let some_field_ty = {
            let Ty::Enum(variants) = self.type_info.ty_pool.get(target_ty)
            else {
                ice!();
            };
            variants[0].1[0]
        };

        let recv_for_some = receiver.clone();
        let build_some = move |this: &mut Self| {
            let field = Value::Clone(Place {
                var: recv_for_some,
                root_ty: receiver_ty,
                projection: vec![Projection::VariantField(wanted, 0)],
            });
            this.make_enum(target_ty, some_name, &[(field, some_field_ty)])
        };
        let build_none =
            move |this: &mut Self| this.make_enum(target_ty, none_name, &[]);

        // `branch_on_discriminant` always runs the success closure on the
        // discriminant-0 path, so swap the two for the `err`/`rejected`
        // direction rather than duplicating the branching logic.
        if from_success {
            self.branch_on_discriminant(
                receiver, target_ty, build_some, build_none,
            )
        } else {
            self.branch_on_discriminant(
                receiver, target_ty, build_none, build_some,
            )
        }
    }

    /// Lowers the `unwrap_or_reject`/`unwrap_or_accept` family: yields the
    /// receiver's success value, or bails out of the *enclosing* function
    /// with a `Verdict`, in the same way the `?` operator does.
    fn intrinsic_early_return(
        &mut self,
        receiver: Var,
        receiver_ty: TyRef,
        early: EarlyReturn,
        arg: Option<(Var, TyRef)>,
    ) -> Value {
        let current_label = self.current_label();
        let lbl_early = self.label_store.wrap_internal(
            current_label,
            Identifier::from("unwrap-or-verdict"),
        );
        let continue_lbl = self.label_store.next(current_label);

        let (success_name, _) = self.enum_variant_names(receiver_ty);
        let return_ty = self.return_type;
        let (accept_name, reject_name) = self.enum_variant_names(return_ty);
        let variant = if early.is_accept() {
            accept_name
        } else {
            reject_name
        };

        let discriminant = self.undropped_tmp();
        self.emit_assign(
            Place::new(discriminant.clone(), TyRef::U8),
            TyRef::U8,
            Value::Discriminant(receiver.clone()),
        );
        self.emit_switch(
            discriminant,
            vec![(0, continue_lbl)],
            Some(lbl_early),
        );

        self.new_block(lbl_early);
        let payload_ty = {
            let Ty::Enum(variants) = self.type_info.ty_pool.get(return_ty)
            else {
                ice!("enclosing function does not return a Verdict");
            };
            let idx = if early.is_accept() { 0 } else { 1 };
            variants[idx].1[0]
        };
        let payload = match &arg {
            Some((var, _)) => Value::Move(var.clone()),
            None => Value::Const(ast::Literal::Unit, TyRef::UNIT),
        };
        let val =
            self.make_enum(return_ty, variant, &[(payload, payload_ty)]);
        // `return_value` drops everything still live, including the
        // receiver (whose failure payload we are discarding here).
        let _ = self.return_value(val);

        self.new_block(continue_lbl);
        // On this path the argument is never consumed, so drop it here.
        // As in `intrinsic_convert`, only the instruction is emitted: the
        // early-return block above already removed it from the compiler's
        // branch-unaware liveness bookkeeping exactly once.
        if let Some((var, ty)) = arg {
            self.emit_drop(Place::new(var, ty), ty);
        }
        Value::Clone(Place {
            var: receiver,
            root_ty: receiver_ty,
            projection: vec![Projection::VariantField(success_name, 0)],
        })
    }

    /// Lowers a call to a compiler-builtin method on Option, Result or
    /// Verdict (see [`FunctionDefinition::Intrinsic`]).
    fn intrinsic_call(
        &mut self,
        intrinsic: Intrinsic,
        receiver: Option<(Value, Type)>,
        arguments: &[Meta<ast::Expr>],
        return_ty: TyRef,
    ) -> Value {
        // Evaluate the receiver and the remaining arguments, in source
        // order, as ordinary expressions so they participate in normal
        // drop tracking.
        //
        // These methods can be reached either as a method call
        // (`x.unwrap_or(d)`) or through the equivalent function-call
        // syntax (`Option.unwrap_or(x, d)`, including via `import
        // Option.unwrap_or`). In the latter case the typechecker
        // resolves them as a plain function, so there is no separate
        // receiver and `self` is simply the first ordinary argument.
        let eval = |this: &mut Self, e: &Meta<ast::Expr>| {
            let ty = this.type_info.type_of(e);
            let ty = this.type_info.convert(&ty);
            let op = this.expr(e);
            (this.assign_to_var(op, ty), ty)
        };

        let ((receiver, receiver_ty), rest) = match receiver {
            Some((val, ty)) => {
                let ty = self.type_info.convert(&ty);
                ((self.assign_to_var(val, ty), ty), arguments)
            }
            None => {
                let (this, rest) = arguments
                    .split_first()
                    .expect("intrinsic methods always take a `self`");
                (eval(self, this), rest)
            }
        };

        let args: Vec<(Var, TyRef)> =
            rest.iter().map(|a| eval(self, a)).collect();

        if let Some(early) = intrinsic.early_return() {
            let arg = if early.takes_argument() {
                Some(args[0].clone())
            } else {
                None
            };
            return self.intrinsic_early_return(
                receiver,
                receiver_ty,
                early,
                arg,
            );
        }

        match intrinsic {
            Intrinsic::IsSuccess => self.branch_on_discriminant(
                receiver,
                TyRef::BOOL,
                |_| Value::Const(ast::Literal::Bool(true), TyRef::BOOL),
                |_| Value::Const(ast::Literal::Bool(false), TyRef::BOOL),
            ),
            Intrinsic::IsFailure => self.branch_on_discriminant(
                receiver,
                TyRef::BOOL,
                |_| Value::Const(ast::Literal::Bool(false), TyRef::BOOL),
                |_| Value::Const(ast::Literal::Bool(true), TyRef::BOOL),
            ),
            Intrinsic::SuccessToOption => self.intrinsic_to_option(
                receiver,
                receiver_ty,
                return_ty,
                true,
            ),
            Intrinsic::FailureToOption => self.intrinsic_to_option(
                receiver,
                receiver_ty,
                return_ty,
                false,
            ),
            Intrinsic::UnwrapOr => {
                let (success_name, _) = self.enum_variant_names(receiver_ty);
                let (default_var, result_ty) = args[0].clone();
                let recv_for_success = receiver.clone();
                let default_for_success = default_var.clone();
                self.branch_on_discriminant(
                    receiver,
                    result_ty,
                    move |this| {
                        // The default is only used on the failure path
                        // below; drop it explicitly here so it isn't
                        // silently leaked whenever the receiver is
                        // actually Some/Ok at runtime (see the doc
                        // comment on `intrinsic_convert` for why this
                        // can't just rely on scope-exit cleanup).
                        this.emit_drop(
                            Place::new(default_for_success, result_ty),
                            result_ty,
                        );
                        Value::Clone(Place {
                            var: recv_for_success,
                            root_ty: receiver_ty,
                            projection: vec![Projection::VariantField(
                                success_name,
                                0,
                            )],
                        })
                    },
                    move |_| Value::Move(default_var),
                )
            }
            Intrinsic::UnwrapOrDefault => {
                // The success type is forced to () by the signature, so
                // the result is always () regardless of which variant the
                // receiver holds - there's no need to even look at it.
                // `receiver` is left otherwise untouched: it stays tracked
                // live and is dropped exactly once by the normal
                // scope-exit cleanup, correctly handling whichever variant
                // it actually holds at runtime (e.g. a real
                // heap-allocated Err/Reject value).
                let _ = receiver;
                Value::Const(ast::Literal::Unit, TyRef::UNIT)
            }
            Intrinsic::OkOr => self.intrinsic_convert(
                receiver,
                receiver_ty,
                return_ty,
                Some(args[0].clone()),
            ),
            Intrinsic::UnwrapOrReject
            | Intrinsic::UnwrapOrRejectDefault
            | Intrinsic::UnwrapOrAccept
            | Intrinsic::UnwrapOrAcceptDefault => {
                unreachable!("handled by early_return above")
            }
        }
    }

    fn literal(&mut self, literal: &Meta<ast::Literal>) -> Value {
        let ty = self.type_info.type_of(literal);
        let ty = self.type_info.convert(&ty);
        Value::Const((**literal).clone(), ty)
    }

    fn return_value(&mut self, val: Value) -> Value {
        let var = self.assign_to_var(val, self.return_type);
        let _ty = self.remove_live_variable(&var);
        for frame in self.stack_slots.clone().iter().rev() {
            for (var, ty) in frame.iter().rev() {
                self.emit_drop(Place::new(var.clone(), *ty), *ty);
            }
        }
        self.emit_return(var);
        Value::Const(ast::Literal::Unit, TyRef::UNIT)
    }

    fn function_call(
        &mut self,
        id: MetaId,
        function: &Meta<ast::Expr>,
        arguments: &Meta<Vec<Meta<ast::Expr>>>,
    ) -> Value {
        match &function.node {
            ast::Expr::Path(p) => {
                let resolved_path = self.type_info.path_kind(p).clone();
                match resolved_path {
                    ResolvedPath::Method {
                        value, signature, ..
                    } => {
                        let op = self.path_value(&value.clone());
                        let ty = signature.parameter_types[0].clone();
                        let func = self.type_info.function(id).clone();
                        self.normalized_function_call(
                            &func,
                            Some((op, ty)),
                            arguments,
                        )
                    }
                    ResolvedPath::Function { .. }
                    | ResolvedPath::StaticMethod { .. } => {
                        let func = self.type_info.function(id).clone();
                        self.normalized_function_call(&func, None, arguments)
                    }
                    ResolvedPath::EnumConstructor { ty: _, variant } => {
                        let ty = self.type_info.type_of(id);
                        let ty = self.type_info.convert(&ty);
                        self.enum_constructor(ty, variant.name, arguments)
                    }
                    ResolvedPath::Value { .. } => ice!(),
                }
            }
            ast::Expr::Access(e, _) => {
                let expr = self.expr(e);
                let ty = self.type_info.type_of(&**e);
                let func = self.type_info.function(id).clone();
                self.normalized_function_call(
                    &func,
                    Some((expr, ty)),
                    arguments,
                )
            }
            _ => ice!(),
        }
    }

    fn normalized_function_call(
        &mut self,
        func: &typechecker::types::Function,
        receiver: Option<(Value, Type)>,
        arguments: &[Meta<ast::Expr>],
    ) -> Value {
        let name = func.name;

        // Intrinsics (compiler-builtin methods on Option/Result) are not
        // backed by a real callee, so they get their own lowering that
        // evaluates the receiver/arguments as ordinary tracked (auto-
        // dropped) local variables, instead of the "the callee drops
        // these" convention used for Runtime/Roto calls below.
        if let FunctionDefinition::Intrinsic(intrinsic) = func.definition {
            let return_type =
                self.type_info.convert(&func.signature.return_type);
            return self.intrinsic_call(
                intrinsic,
                receiver,
                arguments,
                return_type,
            );
        }

        let mut args = Vec::new();
        if let Some((receiver, ty)) = receiver {
            let ty = self.type_info.convert(&ty);
            // This values will be dropped by the callee
            let tmp = self.undropped_tmp();
            self.vars.push((tmp.clone(), ty));

            self.do_assign(Place::new(tmp.clone(), ty), ty, receiver);
            args.push(tmp);
        }

        args.extend(arguments.iter().map(|a| {
            let ty = self.type_info.type_of(a);
            let ty = self.type_info.convert(&ty);
            let op = self.expr(a);

            // These values will be dropped by the callee
            let tmp = self.undropped_tmp();
            self.vars.push((tmp.clone(), ty));

            self.do_assign(Place::new(tmp.clone(), ty), ty, op);
            tmp
        }));

        let mir_signature = ty::Signature {
            parameter_types: func
                .signature
                .parameter_types
                .iter()
                .map(|ty| self.type_info.convert(ty))
                .collect(),
            return_type: self.type_info.convert(&func.signature.return_type),
        };

        match func.definition {
            FunctionDefinition::Runtime(func_ref) => {
                let mut vtables = Vec::new();
                for idx in &self.runtime.get_function(func_ref).vtables {
                    let ty = &func.signature.types[*idx];
                    let ty = self.type_info.convert(ty);
                    vtables.push(ty);
                }

                Value::CallRuntime {
                    func_ref,
                    args,
                    mir_signature,
                    vtables,
                }
            }
            FunctionDefinition::Roto => Value::Call {
                func: name,
                args,
                mir_signature,
            },
            FunctionDefinition::Intrinsic(_) => {
                unreachable!("handled above")
            }
        }
    }

    fn enum_constructor(
        &mut self,
        ty: TyRef,
        variant: Identifier,
        arguments: &[Meta<ast::Expr>],
    ) -> Value {
        let arguments: Vec<_> = arguments
            .iter()
            .map(|a| {
                let ty = self.type_info.type_of(a);
                let ty = self.type_info.convert(&ty);
                (self.expr(a), ty)
            })
            .collect();
        self.make_enum(ty, variant, &arguments)
    }

    fn make_enum(
        &mut self,
        ty: TyRef,
        variant: Identifier,
        arguments: &[(Value, TyRef)],
    ) -> Value {
        let Ty::Enum(variants) = self.type_info.ty_pool.get(ty) else {
            ice!("Not an enum")
        };

        let Some(&(variant_name, _)) =
            variants.iter().find(|v| v.0 == variant)
        else {
            ice!("Could not find variant to construct")
        };

        let to = self.tmp(ty);
        self.emit_set_discriminant(to.clone(), ty, variant_name);
        for (i, (value, field_ty)) in arguments.iter().enumerate() {
            self.do_assign(
                Place {
                    var: to.clone(),
                    root_ty: ty,
                    projection: vec![Projection::VariantField(
                        variant_name,
                        i,
                    )],
                },
                *field_ty,
                value.clone(),
            )
        }
        Value::Move(to)
    }

    fn access(
        &mut self,
        expr: &Meta<ast::Expr>,
        field: &Meta<Identifier>,
    ) -> Value {
        let op = self.expr(expr);
        let ty = self.type_info.type_of(expr);
        let ty = self.type_info.convert(&ty);
        let var = self.assign_to_var(op, ty);
        Value::Clone(Place {
            var,
            root_ty: ty,
            projection: vec![Projection::Field(**field)],
        })
    }

    fn path(&mut self, id: MetaId, path: &Meta<ast::Path>) -> Value {
        let path_kind = self.type_info.path_kind(path).clone();

        match path_kind {
            ResolvedPath::Value(value) => self.path_value(&value),
            ResolvedPath::EnumConstructor { ty: _, variant } => {
                let ty = self.type_info.type_of(id);
                let ty = self.type_info.convert(&ty);
                let to = self.tmp(ty);
                self.emit_set_discriminant(to.clone(), ty, variant.name);
                Value::Move(to)
            }
            _ => ice!("should be rejected by the type checker"),
        }
    }

    fn record(&mut self, id: MetaId, record: &Meta<ast::Record>) -> Value {
        let ty = self.type_info.type_of(id);
        let ty = self.type_info.convert(&ty);

        let to = self.tmp(ty);

        for (s, expr) in &record.fields {
            let op = self.expr(expr);
            let field_ty = self.type_info.type_of(expr);
            let field_ty = self.type_info.convert(&field_ty);
            self.do_assign(
                Place {
                    var: to.clone(),
                    root_ty: ty,
                    projection: vec![Projection::Field(**s)],
                },
                field_ty,
                op,
            );
        }

        Value::Move(to)
    }

    fn not(&mut self, expr: &Meta<ast::Expr>) -> Value {
        let val = self.expr(expr);
        let var = self.assign_to_var(val, TyRef::BOOL);
        Value::Not(var)
    }

    fn negate(&mut self, expr: &Meta<ast::Expr>) -> Value {
        let ty = self.type_info.type_of(expr);
        let ty = self.type_info.convert(&ty);
        let val = self.expr(expr);
        let var = self.assign_to_var(val, ty);
        Value::Negate(var, ty)
    }

    fn list(&mut self, id: MetaId, list: &[Meta<ast::Expr>]) -> Value {
        let ty = self.type_info.type_of(id);
        let ty = self.type_info.convert(&ty);
        let &Ty::List(inner) = self.type_info.ty_pool.get(ty) else {
            ice!()
        };

        let tmp = self.tmp(ty);

        let func_ref = self.find_method(TypeId::of::<ErasedList>(), "new");
        self.emit(Instruction::Assign {
            to: Place {
                var: tmp.clone(),
                root_ty: ty,
                projection: Vec::new(),
            },
            ty,
            value: Value::CallRuntime {
                func_ref,
                args: Vec::new(),
                mir_signature: ty::Signature {
                    parameter_types: Vec::new(),
                    return_type: ty,
                },
                vtables: vec![inner],
            },
        });

        let unit_tmp = self.tmp(TyRef::UNIT);
        for expr in list {
            let list_var = Value::Clone(Place::new(tmp.clone(), ty));
            let list_var = self.assign_to_var(list_var, ty);
            self.remove_live_variable(&list_var);

            let elem = self.expr(expr);
            let elem_ty = self.type_info.type_of(expr);
            let elem_ty = self.type_info.convert(&elem_ty);
            let elem_var = self.undropped_tmp();
            self.vars.push((elem_var.clone(), elem_ty));

            self.do_assign(
                Place::new(elem_var.clone(), elem_ty),
                elem_ty,
                elem,
            );

            let func_ref =
                self.find_method(TypeId::of::<ErasedList>(), "push");
            self.emit(Instruction::Assign {
                to: Place {
                    var: unit_tmp.clone(),
                    root_ty: TyRef::UNIT,
                    projection: Vec::new(),
                },
                ty: TyRef::UNIT,
                value: Value::CallRuntime {
                    func_ref,
                    args: vec![list_var, elem_var],
                    mir_signature: ty::Signature {
                        parameter_types: vec![ty, inner],
                        return_type: TyRef::UNIT,
                    },
                    vtables: Vec::new(),
                },
            });
        }

        Value::Move(tmp)
    }

    fn assign(
        &mut self,
        path: &Meta<ast::Path>,
        expr: &Meta<ast::Expr>,
    ) -> Value {
        let resolved_path = self.type_info.path_kind(path);
        let ResolvedPath::Value(PathValue {
            name,
            kind: _,
            root_ty,
            fields,
        }) = resolved_path.clone()
        else {
            ice!("should be rejected by type checker");
        };
        let root_ty = self.type_info.convert(&root_ty);

        let ty = self.type_info.type_of(expr);
        let ty = self.type_info.convert(&ty);

        let to = Var {
            scope: name.scope,
            kind: VarKind::Explicit(name.ident),
        };

        let place = Place {
            var: to,
            root_ty,
            projection: fields
                .iter()
                .map(|(s, _)| Projection::Field(*s))
                .collect(),
        };

        let val = self.expr(expr);
        let tmp = self.tmp(ty);
        self.do_assign(Place::new(tmp.clone(), ty), ty, val);

        self.emit_drop(place.clone(), ty);

        self.do_assign(place, ty, Value::Move(tmp));

        Value::Const(ast::Literal::Unit, TyRef::UNIT)
    }

    fn compound_assign(&mut self, c: &ast::CompoundAssign) -> Value {
        let op = match c.op {
            CompoundAssignOp::Add => ast::BinOp::Add,
            CompoundAssignOp::Sub => ast::BinOp::Sub,
            CompoundAssignOp::Mul => ast::BinOp::Mul,
            CompoundAssignOp::Div => ast::BinOp::Div,
            CompoundAssignOp::Mod => ast::BinOp::Mod,
        };
        let left = Meta {
            id: c.path_expr_id,
            node: Expr::Path(c.path.clone()),
        };
        let bin_expr = Meta {
            id: c.binop_id,
            node: Expr::BinOp(Box::new(left), op, c.expr.clone()),
        };
        self.assign(&c.path, &bin_expr)
    }

    fn binop(
        &mut self,
        l: &Meta<ast::Expr>,
        binop: &ast::BinOp,
        r: &Meta<ast::Expr>,
    ) -> Value {
        let l_ty = self.type_info.type_of(l);
        let r_ty = self.type_info.type_of(r);

        if *binop == ast::BinOp::Eq {
            let l_ty = self.type_info.convert(&l_ty);
            let r_ty = self.type_info.convert(&r_ty);
            let left = self.expr(l);
            let left = self.assign_to_var(left, l_ty);
            let right = self.expr(r);
            let right = self.assign_to_var(right, r_ty);
            return Value::BinOp {
                left,
                binop: ast::BinOp::Eq,
                ty: l_ty,
                right,
            };
        }

        if *binop == ast::BinOp::Ne {
            let l_ty = self.type_info.convert(&l_ty);
            let r_ty = self.type_info.convert(&r_ty);
            let left = self.expr(l);
            let left = self.assign_to_var(left, l_ty);
            let right = self.expr(r);
            let right = self.assign_to_var(right, r_ty);
            return Value::BinOp {
                left,
                binop: ast::BinOp::Ne,
                ty: l_ty,
                right,
            };
        }

        if l_ty == Type::string() {
            return self.binop_str(l, binop, r);
        }

        if l_ty == Type::ip_addr() {
            return self.binop_ip_addr(l, binop, r);
        }

        if self.type_info.is_list_type(&l_ty) {
            return self.binop_list(l_ty, l, binop, r);
        }

        if *binop == ast::BinOp::And {
            return self.binop_and(l, r);
        }

        if *binop == ast::BinOp::Or {
            return self.binop_or(l, r);
        }

        let l_ty = self.type_info.convert(&l_ty);
        let r_ty = self.type_info.convert(&r_ty);

        let l = self.expr(l);
        let l = self.assign_to_var(l, l_ty);

        let r = self.expr(r);
        let r = self.assign_to_var(r, r_ty);

        Value::BinOp {
            left: l,
            binop: *binop,
            ty: l_ty,
            right: r,
        }
    }

    fn binop_str(
        &mut self,
        l: &Meta<ast::Expr>,
        binop: &ast::BinOp,
        r: &Meta<ast::Expr>,
    ) -> Value {
        let type_id = TypeId::of::<crate::RotoString>();
        match binop {
            ast::BinOp::Add => self.desugared_binop(
                type_id,
                "append",
                Type::string(),
                (l, Type::string()),
                (r, Type::string()),
            ),
            _ => {
                ice!("Operator {binop} is not implemented for String")
            }
        }
    }

    fn binop_ip_addr(
        &mut self,
        l: &Meta<ast::Expr>,
        binop: &ast::BinOp,
        r: &Meta<ast::Expr>,
    ) -> Value {
        match binop {
            ast::BinOp::Div => {
                let type_id = TypeId::of::<Prefix>();
                self.desugared_binop(
                    type_id,
                    "new",
                    Type::prefix(),
                    (l, Type::ip_addr()),
                    (r, Type::u8()),
                )
            }
            _ => {
                ice!("Operator {binop} is not implemented for IpAddr")
            }
        }
    }

    fn binop_list(
        &mut self,
        ty: Type,
        l: &Meta<ast::Expr>,
        binop: &ast::BinOp,
        r: &Meta<ast::Expr>,
    ) -> Value {
        match binop {
            ast::BinOp::Add => self.desugared_binop(
                TypeId::of::<ErasedList>(),
                "concat",
                ty.clone(),
                (l, ty.clone()),
                (r, ty.clone()),
            ),
            _ => {
                ice!("Operator {binop} is not implemented for List")
            }
        }
    }

    fn binop_and(
        &mut self,
        l: &Meta<ast::Expr>,
        r: &Meta<ast::Expr>,
    ) -> Value {
        self.shortcircuit_binop(l, r, "and_other", 1)
    }

    fn binop_or(
        &mut self,
        l: &Meta<ast::Expr>,
        r: &Meta<ast::Expr>,
    ) -> Value {
        self.shortcircuit_binop(l, r, "or_other", 0)
    }

    fn shortcircuit_binop(
        &mut self,
        l: &Meta<ast::Expr>,
        r: &Meta<ast::Expr>,
        lbl_other: &str,
        other_if: usize,
    ) -> Value {
        let current_label = self.current_label();
        let lbl_cont = self.label_store.next(current_label);
        let lbl_other = self
            .label_store
            .wrap_internal(current_label, Identifier::from(lbl_other));

        // The variable that we store the result of this operation in.
        // We can return before the end of this operation so this cannot
        // be added to the stack slots yet.
        let tmp = self.undropped_tmp();

        // Create a new stack slot for the left-hand side.
        // This is not strictly necessary, but it makes a nice symmetry with
        // the right-hand side.
        self.stack_slots.push(Vec::new());

        let val = self.expr(l);

        self.do_assign(
            Place::new(tmp.clone(), TyRef::BOOL),
            TyRef::BOOL,
            val,
        );

        // Drop all variables for the left-hand side
        let to_drop = self.stack_slots.pop().unwrap();
        for (var, ty) in to_drop.into_iter().rev() {
            self.emit_drop(Place::new(var, ty), ty);
        }

        self.emit_switch(
            tmp.clone(),
            vec![(other_if, lbl_other)],
            Some(lbl_cont),
        );

        self.new_block(lbl_other);

        // Create a new stack slot of the right-hand side. This is necessary
        // because the right-hand side might not be executed. Therefore, we
        // can only drop the variables in it if it has been executed.
        self.stack_slots.push(Vec::new());

        let val = self.expr(r);
        self.do_assign(
            Place::new(tmp.clone(), TyRef::BOOL),
            TyRef::BOOL,
            val,
        );

        // Drop all variables in the right-hand side.
        let to_drop = self.stack_slots.pop().unwrap();
        for (var, ty) in to_drop.into_iter().rev() {
            self.emit_drop(Place::new(var, ty), ty);
        }

        // Finally, we know that `tmp` has been initialized and can be dropped.
        self.add_live_variable(tmp.clone(), TyRef::BOOL);

        self.emit_jump(lbl_cont);
        self.new_block(lbl_cont);

        Value::Move(tmp)
    }

    fn desugared_binop(
        &mut self,
        kind: TypeId,
        name: &str,
        return_type: Type,
        (l, l_ty): (&Meta<ast::Expr>, Type),
        (r, r_ty): (&Meta<ast::Expr>, Type),
    ) -> Value {
        let func_ref = self.find_method(kind, name);

        let l = self.expr(l);
        let l_ty = self.type_info.convert(&l_ty);
        let l = self.assign_to_var(l, l_ty);
        let r = self.expr(r);
        let r_ty = self.type_info.convert(&r_ty);
        let r = self.assign_to_var(r, r_ty);

        let return_type = self.type_info.convert(&return_type);
        let tmp = self.tmp(return_type);
        let mir_signature = ty::Signature {
            parameter_types: vec![l_ty, r_ty],
            return_type,
        };
        let val = self.call_runtime(
            func_ref,
            Vec::new(),
            mir_signature,
            vec![l, r],
        );
        self.do_assign(
            Place::new(tmp.clone(), return_type),
            return_type,
            val,
        );

        Value::Move(tmp)
    }

    fn call_runtime(
        &mut self,
        func_ref: RuntimeFunctionRef,
        vtables: Vec<TyRef>,
        mir_signature: ty::Signature,
        args: Vec<Var>,
    ) -> Value {
        for var in &args {
            self.remove_live_variable(var);
        }
        Value::CallRuntime {
            func_ref,
            args,
            mir_signature,
            vtables,
        }
    }

    fn if_else(
        &mut self,
        id: MetaId,
        condition: &Meta<ast::Expr>,
        then: &Meta<ast::Block>,
        r#else: &Option<Meta<ast::Block>>,
    ) -> Value {
        let examinee = self.expr(condition);
        let examinee = self.assign_to_var(examinee, TyRef::BOOL);

        let current_label = self.current_label();
        let lbl_cont = self.label_store.next(current_label);
        let lbl_then = self
            .label_store
            .wrap_internal(current_label, Identifier::from("if-then"));
        let lbl_else = self
            .label_store
            .wrap_internal(current_label, Identifier::from("if-else"));

        let branches = vec![(1, lbl_then)];

        self.emit_switch(
            examinee,
            branches,
            Some(if r#else.is_some() { lbl_else } else { lbl_cont }),
        );

        self.new_block(lbl_then);
        let op = self.block(then);

        let ty = self.type_info.type_of(id);
        let ty = self.type_info.convert(&ty);

        let res = self.undropped_tmp();
        self.emit_assign(Place::new(res.clone(), ty), ty, op);

        self.emit_jump(lbl_cont);

        if let Some(r#else) = r#else {
            self.new_block(lbl_else);
            let op = self.block(r#else);
            self.emit_assign(Place::new(res.clone(), ty), ty, op);
            self.emit_jump(lbl_cont);
        }
        self.new_block(lbl_cont);
        self.add_live_variable(res.clone(), ty);
        Value::Move(res)
    }

    fn r#while(
        &mut self,
        condition: &Meta<ast::Expr>,
        block: &Meta<ast::Block>,
    ) -> Value {
        let current_label = self.current_label();
        let lbl_cont = self.label_store.next(current_label);
        let lbl_condition = self.label_store.wrap_internal(
            current_label,
            Identifier::from("while-condition"),
        );
        let lbl_body = self
            .label_store
            .wrap_internal(current_label, Identifier::from("while-body"));

        self.emit_jump(lbl_condition);

        self.new_block(lbl_condition);

        let examinee = self.expr(condition);
        let examinee = self.assign_to_var(examinee, TyRef::BOOL);

        self.emit_switch(examinee, vec![(1, lbl_body)], Some(lbl_cont));

        self.new_block(lbl_body);
        let val = self.block(block);
        let _ = self.assign_to_var(val, TyRef::UNIT);
        self.emit_jump(lbl_condition);

        self.new_block(lbl_cont);
        Value::Const(Literal::Unit, TyRef::UNIT)
    }

    fn r#for(
        &mut self,
        name: &Meta<Identifier>,
        expr: &Meta<ast::Expr>,
        body: &Meta<ast::Block>,
    ) -> Value {
        let lbl_current = self.current_label();
        let lbl_cont = self.label_store.next(lbl_current);

        // We will be creating the following blocks in pseudocode:
        //
        // lbl_current:
        //   ...
        //   idx = 0
        //   jump lbl_condition
        //
        // lbl_increment:
        //   idx += 1
        //   jump lbl_condition
        //
        // lbl_condition:
        //   opt_elem = list.get(idx);
        //   if some:
        //       jump lbl_body
        //   else:
        //       jump lbl_cont
        //
        // lbl_body:
        //   jump lbl_increment

        let lbl_increment = self
            .label_store
            .wrap_internal(lbl_current, Identifier::from("for-increment"));
        let lbl_condition = self
            .label_store
            .wrap_internal(lbl_current, Identifier::from("for-condition"));
        let lbl_body = self
            .label_store
            .wrap_internal(lbl_current, Identifier::from("for-body"));

        let elem_ty = self.type_info.type_of(name);
        let opt_elem_ty = Type::option(&elem_ty);

        let elem_ty = self.type_info.convert(&elem_ty);
        let opt_elem_ty = self.type_info.convert(&opt_elem_ty);

        let opt_elem_var = self.undropped_tmp();
        self.vars.push((opt_elem_var.clone(), opt_elem_ty));

        // This is the setup for the iteration. We evaluate the expression for
        // the list and set the index to 0.
        let list_ty = self.type_info.type_of(expr);
        let list_ty = self.type_info.convert(&list_ty);
        let list_value = self.expr(expr);
        let list_var = self.assign_to_var(list_value, list_ty);
        let index_var = self.assign_to_var(
            Value::Const(Literal::Integer(0, Some(IntType::U64)), TyRef::U64),
            TyRef::U64,
        );

        // We do not want to increment on the first iteration, so we jump to the condition.
        self.emit_jump(lbl_condition);

        // This block increments the index variable and jumps to the condition.
        {
            self.new_block(lbl_increment);
            let one_var = self.assign_to_var(
                Value::Const(
                    Literal::Integer(1, Some(IntType::U64)),
                    TyRef::U64,
                ),
                TyRef::U64,
            );
            let new_index = Value::BinOp {
                left: index_var.clone(),
                binop: ast::BinOp::Add,
                ty: TyRef::U64,
                right: one_var.clone(),
            };
            self.emit_assign(
                Place::new(index_var.clone(), TyRef::U64),
                TyRef::U64,
                new_index,
            );
            self.emit_jump(lbl_condition);
        }

        // This block gets the value at the given index and breaks out of the loop if the
        // result is None.
        {
            self.new_block(lbl_condition);

            let func_ref =
                self.find_method(TypeId::of::<ErasedList>(), "get");
            let new_list_var = self.assign_to_var(
                Value::Clone(Place::new(list_var, list_ty)),
                list_ty,
            );
            self.remove_live_variable(&new_list_var);
            let mir_signature = ty::Signature {
                parameter_types: vec![list_ty, TyRef::U64],
                return_type: opt_elem_ty,
            };
            self.emit(Instruction::Assign {
                to: Place::new(opt_elem_var.clone(), opt_elem_ty),
                ty: opt_elem_ty,
                value: Value::CallRuntime {
                    func_ref,
                    args: vec![new_list_var, index_var],
                    mir_signature,
                    vtables: Vec::new(),
                },
            });

            let discriminant = self.undropped_tmp();
            self.emit_assign(
                Place::new(discriminant.clone(), TyRef::U8),
                TyRef::U8,
                Value::Discriminant(opt_elem_var.clone()),
            );
            self.emit_switch(
                discriminant,
                vec![(0, lbl_body)],
                Some(lbl_cont),
            );
        }

        self.new_block(lbl_body);
        self.stack_slots.push(Vec::new());
        let resolved_name = self.type_info.resolved_name(name);
        let elem_var = Var {
            scope: resolved_name.scope,
            kind: VarKind::Explicit(resolved_name.ident),
        };
        self.vars.push((elem_var.clone(), elem_ty));

        let slot = self.stack_slots.last_mut().unwrap();

        slot.push((opt_elem_var.clone(), opt_elem_ty));
        slot.push((elem_var.clone(), elem_ty));

        self.do_assign(
            Place::new(elem_var, elem_ty),
            elem_ty,
            // This clone is unfortunate, but the best that the IR currently allows.
            // This could become a move in the future.
            Value::Clone(Place {
                var: opt_elem_var,
                root_ty: opt_elem_ty,
                projection: vec![Projection::VariantField("Some".into(), 0)],
            }),
        );

        let val = self.block(body);
        let _ = self.assign_to_var(val, TyRef::UNIT);

        let to_drop = self.stack_slots.pop().unwrap();
        for (var, ty) in to_drop.into_iter().rev() {
            self.emit_drop(Place::new(var, ty), ty);
        }

        self.emit_jump(lbl_increment);

        self.new_block(lbl_cont);
        Value::Const(Literal::Unit, TyRef::UNIT)
    }

    fn path_value(&mut self, path_value: &PathValue) -> Value {
        let PathValue {
            name,
            kind,
            root_ty,
            fields,
        } = path_value;

        let root_ty = self.type_info.convert(root_ty);
        match kind {
            ValueKind::Local => {
                let var = Var {
                    scope: name.scope,
                    kind: VarKind::Explicit(name.ident),
                };

                let projection =
                    fields.iter().map(|f| Projection::Field(f.0)).collect();

                Value::Clone(Place {
                    var,
                    root_ty,
                    projection,
                })
            }
            ValueKind::Constant => {
                if !fields.is_empty() {
                    panic!("Getting fields of constants not supported yet")
                }
                Value::Constant(*name, root_ty)
            }
            ValueKind::Context(x) => Value::Context(*x),
        }
    }

    fn add_live_variable(&mut self, var: Var, ty: TyRef) {
        self.vars.push((var.clone(), ty));
        self.stack_slots.last_mut().unwrap().push((var, ty));
    }

    fn remove_live_variable(&mut self, var: &Var) -> TyRef {
        let slot = self.stack_slots.last_mut().unwrap();
        if let Some(i) = slot.iter().position(|v| &v.0 == var) {
            slot.remove(i).1
        } else {
            let printer = IrPrinter {
                type_info: self.type_info,
                label_store: self.label_store,
                scope: None,
            };
            let var = var.print(&printer);
            ice!("Variable wasn't live: {var:?}")
        }
    }

    fn find_method(
        &self,
        type_id: TypeId,
        ident: &str,
    ) -> RuntimeFunctionRef {
        let ty = self
            .runtime
            .types()
            .iter()
            .find(|t| t.type_id() == type_id)
            .unwrap();

        let name = ty.name();

        let scope = self
            .type_info
            .scope_graph
            .get_declaration(name)
            .scope
            .unwrap();

        let dec = self.type_info.scope_graph.get_declaration(ResolvedName {
            scope,
            ident: ident.into(),
        });

        let DeclarationKind::Method(Some(f)) = dec.kind else {
            ice!();
        };

        let FunctionDefinition::Runtime(r) = f.definition else {
            ice!();
        };

        r
    }

    fn do_assign(&mut self, to: Place, ty: TyRef, val: Value) {
        if let Value::Move(var) = &val {
            self.remove_live_variable(var);
        }
        self.emit_assign(to, ty, val);
    }

    fn f_string(&mut self, parts: &[Meta<ast::FStringPart>]) -> Value {
        let make_string = |s: String| -> Value {
            Value::Const(Literal::String(s), TyRef::STRING)
        };

        let string_val = make_string("".into());
        let string = self.assign_to_var(string_val, TyRef::STRING);

        let type_id = TypeId::of::<crate::RotoString>();
        let func_ref = self.find_method(type_id, "append");

        for part in parts {
            let id = part.id;
            let new_string = match &part.node {
                ast::FStringPart::String(s) => make_string(s.clone()),
                ast::FStringPart::Expr(expr) => {
                    let val = self.expr(expr);
                    let ty = self.type_info.type_of(expr);

                    // Get the function that the type checker determined we
                    // should call, this will be the `to_string` method on
                    // the type.
                    let func = self.type_info.function(id).clone();
                    self.normalized_function_call(&func, Some((val, ty)), &[])
                }
            };

            let new_string = self.assign_to_var(new_string, TyRef::STRING);

            let mir_signature = ty::Signature {
                parameter_types: vec![TyRef::STRING, TyRef::STRING],
                return_type: TyRef::STRING,
            };

            let val = self.call_runtime(
                func_ref,
                Vec::new(),
                mir_signature,
                vec![string.clone(), new_string],
            );

            self.stack_slots
                .last_mut()
                .unwrap()
                .push((string.clone(), TyRef::STRING));
            self.do_assign(
                Place::new(string.clone(), TyRef::STRING),
                TyRef::STRING,
                val,
            );
        }

        Value::Move(string)
    }
}

impl Lowerer<'_> {
    fn emit(&mut self, instruction: Instruction) {
        self.blocks
            .last_mut()
            .unwrap()
            .instructions
            .push(instruction)
    }

    fn emit_assign(&mut self, to: Place, ty: TyRef, value: Value) {
        self.emit(Instruction::Assign { to, ty, value });
    }

    fn emit_set_discriminant(
        &mut self,
        to: Var,
        ty: TyRef,
        variant: Identifier,
    ) {
        self.emit(Instruction::SetDiscriminant { to, ty, variant })
    }

    fn emit_return(&mut self, value: Var) {
        self.emit(Instruction::Return { var: value });
    }

    fn emit_jump(&mut self, label_ref: LabelRef) {
        self.emit(Instruction::Jump(label_ref))
    }

    fn emit_switch(
        &mut self,
        examinee: Var,
        branches: Vec<(usize, LabelRef)>,
        default: Option<LabelRef>,
    ) {
        self.emit(Instruction::Switch {
            examinee,
            branches,
            default,
        })
    }

    fn emit_drop(&mut self, val: Place, ty: TyRef) {
        self.emit(Instruction::Drop { val, ty })
    }
}
