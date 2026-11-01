//! Type predicates, ported from roblox-ts 3.0.0's `util/types.ts` and `isMethod.ts`, and the global symbols that
//! identify macros and built-in classes.

use std::collections::{HashMap, HashSet};

use tsrs_ast::{Kind, Node, Symbol, SymbolFlags};
use tsrs_checker::{Checker, LiteralValue, ObjectFlags, SignatureKind, Type, TypeFlags};
use tsrs_core::P;

use super::Extractor;
use crate::facts::{Constant, Falsy, Receiver};
use crate::macros::{Macro, MacroKind};

/// Global symbols, resolved once. A name the program does not declare is absent and never matches.
pub(super) struct Globals {
    calls: HashMap<P<Symbol>, Macro>,
    constructors: HashMap<P<Symbol>, Macro>,
    methods: HashMap<P<Symbol>, Macro>,
    pub identifier_macro: Option<P<Symbol>>,
    arrays: Vec<P<Symbol>>,
    sets: Vec<P<Symbol>>,
    maps: Vec<P<Symbol>>,
    iterable_function: Option<P<Symbol>>,
    lua_tuple: Option<P<Symbol>>,
    pub vector3: Option<P<Symbol>>,
    pub color3: Option<P<Symbol>>,
}

impl Globals {
    pub fn new(c: &mut Checker) -> Globals {
        let mut resolve = |name: &str, meaning: SymbolFlags| c.resolve_name_exported(name, None, meaning, false);
        let mut globals = Globals {
            calls: HashMap::new(),
            constructors: HashMap::new(),
            methods: HashMap::new(),
            identifier_macro: None,
            arrays: Vec::new(),
            sets: Vec::new(),
            maps: Vec::new(),
            iterable_function: None,
            lua_tuple: None,
            vector3: None,
            color3: None,
        };
        for &m in Macro::ALL {
            match m.kind() {
                MacroKind::Call => {
                    if let Some(s) = resolve(m.name(), SymbolFlags::Function) {
                        globals.calls.insert(s, m);
                    }
                }
                MacroKind::Constructor => {
                    if let Some(s) = resolve(m.name(), SymbolFlags::Interface) {
                        globals.constructors.insert(s, m);
                    }
                }
                MacroKind::Identifier => globals.identifier_macro = resolve(m.name(), SymbolFlags::All),
                // roblox-ts keys a property call macro by the method's own symbol, which the callee's type carries.
                MacroKind::Method => {
                    if let Some(member) = resolve(m.owner(), SymbolFlags::Interface)
                        .and_then(|owner| owner.members())
                        .and_then(|members| members.lookup(m.name()))
                    {
                        globals.methods.insert(member, m);
                    }
                }
                MacroKind::Crucible => {}
            }
        }
        let mut all = |names: &[&str]| names.iter().filter_map(|n| resolve(n, SymbolFlags::All)).collect::<Vec<_>>();
        globals.arrays = all(&["Array", "ReadonlyArray", "ReadVoxelsArray", "TemplateStringsArray"]);
        globals.sets = all(&["Set", "ReadonlySet", "WeakSet"]);
        globals.maps = all(&["Map", "ReadonlyMap", "WeakMap"]);
        globals.iterable_function = resolve("IterableFunction", SymbolFlags::All);
        globals.vector3 = resolve("Vector3", SymbolFlags::Type);
        globals.color3 = resolve("Color3", SymbolFlags::Type);
        globals.lua_tuple = resolve("LuaTuple", SymbolFlags::All).and_then(|s| {
            let t = c.get_declared_type_of_symbol_exported(s);
            c.get_property_of_type_exported(t, "_nominal_LuaTuple")
        });
        globals
    }

    pub fn call_macro(&self, s: P<Symbol>) -> Option<Macro> {
        self.calls.get(&s).copied()
    }

    pub fn constructor_macro(&self, s: P<Symbol>) -> Option<Macro> {
        self.constructors.get(&s).copied()
    }

    pub fn method_macro(&self, s: P<Symbol>) -> Option<Macro> {
        self.methods.get(&s).copied()
    }
}

pub(super) fn constant(v: LiteralValue) -> Option<Constant> {
    match v {
        LiteralValue::Number(n) => Some(Constant::Number(n.0)),
        LiteralValue::String(s) => Some(Constant::String(s.into())),
        _ => None,
    }
}

pub(super) fn is_any(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Any)
}

pub(super) fn is_string(_: &mut Extractor, t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::StringLike)
}

pub(super) fn is_integer_literal(_: &mut Extractor, t: P<Type>) -> bool {
    literal_number(t).is_some_and(|v| v.fract() == 0.0)
}

fn is_number(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::NumberLike)
}

pub(super) fn literal_number(t: P<Type>) -> Option<f64> {
    match t.flags().intersects(TypeFlags::NumberLiteral).then(|| t.as_literal_type().value()) {
        Some(Some(LiteralValue::Number(n))) => Some(n.0),
        _ => None,
    }
}

pub(super) fn literal_string(t: P<Type>) -> Option<&'static str> {
    match t.flags().intersects(TypeFlags::StringLiteral).then(|| t.as_literal_type().value()) {
        Some(Some(LiteralValue::String(s))) => Some(s),
        _ => None,
    }
}

pub(super) fn literal_bool(t: P<Type>) -> Option<bool> {
    match t.flags().intersects(TypeFlags::BooleanLiteral).then(|| t.as_literal_type().value()) {
        Some(Some(LiteralValue::Boolean(b))) => Some(b),
        _ => None,
    }
}

fn is_skippable(k: Kind) -> bool {
    matches!(
        k,
        Kind::NonNullExpression
            | Kind::ParenthesizedExpression
            | Kind::AsExpression
            | Kind::TypeAssertionExpression
            | Kind::SatisfiesExpression
    )
}

/// roblox-ts's `skipDownwards`.
pub(super) fn skip_outer(mut n: P<Node>) -> P<Node> {
    while is_skippable(n.kind()) {
        n = n.expression().unwrap();
    }
    n
}

/// roblox-ts's `skipUpwards`.
fn skip_upwards(mut n: P<Node>) -> P<Node> {
    while let Some(p) = n.parent() {
        if !is_skippable(p.kind()) {
            break;
        }
        n = p;
    }
    n
}

type Predicate = fn(&mut Extractor, P<Type>) -> bool;

impl Extractor {
    fn constraint(&mut self, t: P<Type>) -> P<Type> {
        self.checker.get_base_constraint_of_type_exported(t).unwrap_or(t)
    }

    fn base_types(&mut self, t: P<Type>) -> Vec<P<Type>> {
        if !t.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut stack = vec![t];
        let mut seen = HashSet::new();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            for &base in self.checker.get_base_types_exported(current) {
                out.push(base);
                if base.object_flags().intersects(ObjectFlags::ClassOrInterface) {
                    stack.push(base);
                }
            }
        }
        out
    }

    /// roblox-ts's `isDefinitelyType`: every union member, or any intersection member or base type, satisfies `f`.
    pub(super) fn definitely(&mut self, t: P<Type>, f: Predicate) -> bool {
        let t = self.constraint(t);
        self.definitely_inner(t, f)
    }

    fn definitely_inner(&mut self, t: P<Type>, f: Predicate) -> bool {
        if t.flags().intersects(TypeFlags::Union) {
            return t.types().iter().all(|&m| self.definitely_inner(m, f));
        }
        if t.flags().intersects(TypeFlags::Intersection) {
            return t.types().iter().any(|&m| self.definitely_inner(m, f));
        }
        for base in self.base_types(t) {
            if self.definitely_inner(base, f) {
                return true;
            }
        }
        f(self, t)
    }

    /// roblox-ts's `isPossiblyType` for strings, which the `+` lowering asks.
    pub(super) fn possibly_string(&mut self, t: P<Type>) -> bool {
        if t.flags().intersects(TypeFlags::AnyOrUnknown | TypeFlags::TypeVariable) {
            return false;
        }
        let t = self.constraint(t);
        self.possibly_string_inner(t)
    }

    fn possibly_string_inner(&mut self, t: P<Type>) -> bool {
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return t.types().iter().any(|&m| self.possibly_string_inner(m));
        }
        t.flags().intersects(TypeFlags::StringLike)
    }

    /// The JavaScript-falsy values a type may hold: roblox-ts's `isPossiblyType` for `0`, NaN, `""`, `false` and
    /// `undefined`, in one walk.
    pub(super) fn falsy(&mut self, t: P<Type>) -> Falsy {
        let t = self.constraint(t);
        self.falsy_inner(t)
    }

    fn falsy_inner(&mut self, t: P<Type>) -> Falsy {
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return t.types().iter().fold(Falsy::NONE, |acc, &m| acc | self.falsy_inner(m));
        }
        let mut falsy = Falsy::NONE;
        for base in self.base_types(t) {
            falsy = falsy | self.falsy_inner(base);
        }
        if t.flags().intersects(TypeFlags::TypeVariable | TypeFlags::AnyOrUnknown) {
            return Falsy::ALL;
        }
        // `{}` and other empty object types hold anything except nil.
        if self.is_defined_type(t) {
            return falsy | Falsy::ALL.without(Falsy::NIL);
        }
        let flags = t.flags();
        if flags.intersects(TypeFlags::Undefined | TypeFlags::Void | TypeFlags::Null) {
            falsy = falsy | Falsy::NIL;
        }
        if literal_bool(t) == Some(false) || flags.intersects(TypeFlags::Boolean) {
            falsy = falsy | Falsy::FALSE;
        }
        if is_number(t) {
            match literal_number(t) {
                Some(0.0) => falsy = falsy | Falsy::ZERO,
                Some(_) => {}
                None => falsy = falsy | Falsy::ZERO | Falsy::NAN,
            }
        }
        if flags.intersects(TypeFlags::StringLike) {
            let empty = match literal_string(t) {
                Some(s) => s.is_empty(),
                None if flags.intersects(TypeFlags::TemplateLiteral) => {
                    t.as_template_literal_type().texts().iter().all(|s| s.is_empty())
                }
                None => true,
            };
            if empty {
                falsy = falsy | Falsy::EMPTY_STRING;
            }
        }
        falsy
    }

    fn is_defined_type(&mut self, t: P<Type>) -> bool {
        t.flags() == TypeFlags::Object
            && self.checker.get_properties_of_type_exported(t).is_empty()
            && self.checker.get_signatures_of_type_exported(t, SignatureKind::Call).is_empty()
            && self.checker.get_signatures_of_type_exported(t, SignatureKind::Construct).is_empty()
            && self.checker.get_index_infos_of_type_exported(t).is_empty()
    }

    fn symbol_in(t: P<Type>, symbols: &[P<Symbol>]) -> bool {
        t.symbol().is_some_and(|s| symbols.contains(&s))
    }

    fn is_array(x: &mut Extractor, t: P<Type>) -> bool {
        !is_any(t)
            && (t.is_tuple_type() || x.checker.is_array_like_type_exported(t) || Self::symbol_in(t, &x.globals.arrays))
    }

    fn is_set(x: &mut Extractor, t: P<Type>) -> bool {
        Self::symbol_in(t, &x.globals.sets)
    }

    fn is_map(x: &mut Extractor, t: P<Type>) -> bool {
        Self::symbol_in(t, &x.globals.maps)
    }

    fn is_lua_tuple(&mut self, t: P<Type>) -> bool {
        let Some(nominal) = self.globals.lua_tuple else { return false };
        self.checker.get_property_of_type_exported(t, "_nominal_LuaTuple") == Some(nominal)
    }

    fn is_iterable_function(x: &mut Extractor, t: P<Type>) -> bool {
        x.globals.iterable_function.is_some() && t.symbol() == x.globals.iterable_function
    }

    fn is_lua_tuple_iterable_function(x: &mut Extractor, t: P<Type>) -> bool {
        Self::is_iterable_function(x, t)
            && t.object_flags().intersects(ObjectFlags::Reference)
            && x.checker.get_type_arguments_exported(t).first().is_some_and(|&step| x.is_lua_tuple(step))
    }

    /// The class of a value over its non-nullable part, and whether it is a union of members that lower differently.
    /// Every site asks this one question, so two sites on one node never disagree.
    pub(super) fn value_class(&mut self, t: P<Type>) -> (Receiver, bool) {
        let t = self.checker.get_non_nullable_type(t);
        // `never` is assignable to every array type, and it is what is left of `void` and `undefined`.
        let receiver = if t.flags().intersects(TypeFlags::Never) {
            Receiver::Other
        } else if self.is_lua_tuple(t) {
            Receiver::LuaTuple
        } else if self.definitely(t, Self::is_array) {
            Receiver::Array
        } else if self.definitely(t, Self::is_map) {
            Receiver::Map
        } else if self.definitely(t, Self::is_set) {
            Receiver::Set
        } else if self.definitely(t, is_string) {
            Receiver::String
        } else if self.definitely(t, Self::is_lua_tuple_iterable_function) {
            Receiver::LuaTupleIterFn
        } else if self.definitely(t, Self::is_iterable_function) {
            Receiver::IterFn
        } else {
            Receiver::Other
        };
        if receiver != Receiver::Other {
            return (receiver, false);
        }
        let constrained = self.constraint(t);
        let parts: Vec<P<Type>> = if constrained.flags().intersects(TypeFlags::Union) {
            constrained.types().to_vec()
        } else {
            vec![constrained]
        };
        let mut classes = HashSet::new();
        for part in parts {
            let class = if Self::is_array(self, part) {
                Receiver::Array
            } else if Self::is_map(self, part) {
                Receiver::Map
            } else if Self::is_set(self, part) {
                Receiver::Set
            } else if is_string(self, part) {
                Receiver::String
            } else if Self::is_lua_tuple_iterable_function(self, part) {
                Receiver::LuaTupleIterFn
            } else if Self::is_iterable_function(self, part) {
                Receiver::IterFn
            } else {
                Receiver::Other
            };
            classes.insert(class);
        }
        (receiver, classes.len() > 1)
    }

    pub(super) fn possibly_roblox(&mut self, t: P<Type>) -> bool {
        let t = self.constraint(t);
        self.possibly_roblox_inner(t)
    }

    fn possibly_roblox_inner(&mut self, t: P<Type>) -> bool {
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return t.types().iter().any(|&m| self.possibly_roblox_inner(m));
        }
        if self.base_types(t).into_iter().any(|base| self.possibly_roblox_inner(base)) {
            return true;
        }
        if t.flags().intersects(TypeFlags::TypeVariable | TypeFlags::AnyOrUnknown) || self.is_defined_type(t) {
            return true;
        }
        t.symbol().is_some_and(|s| {
            s.declarations().iter().any(|&d| {
                tsrs_ast::get_source_file_of_node(d)
                    .is_some_and(|f| f.file_name().replace('\\', "/").contains("/node_modules/@rbxts/types/"))
            })
        })
    }

    pub(super) fn first_defined_symbol(&mut self, t: P<Type>) -> Option<P<Symbol>> {
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            let undefined = self.checker.get_undefined_symbol();
            return t.types().iter().filter_map(|m| m.symbol()).find(|&s| s != undefined);
        }
        t.symbol()
    }

    /// roblox-ts's `isMethodDeclaration`.
    fn is_method_declaration(&mut self, node: P<Node>) -> bool {
        if !tsrs_ast::is_function_like(node) {
            return false;
        }
        if let Some(&first) = node.parameters().first()
            && let Some(name) = first.name()
            && name.kind() == Kind::Identifier
            && name.as_identifier().text() == "this"
        {
            let t = self.checker.get_type_at_location(name);
            return !t.flags().intersects(TypeFlags::Void);
        }
        match node.kind() {
            Kind::MethodDeclaration | Kind::MethodSignature => true,
            Kind::FunctionExpression => skip_upwards(node).parent().is_some_and(|p| {
                p.kind() == Kind::PropertyAssignment
                    && skip_upwards(p).parent().is_some_and(|g| g.kind() == Kind::ObjectLiteralExpression)
            }),
            _ => false,
        }
    }

    /// roblox-ts's `isMethod`: whether any call signature takes `self`, and whether any does not.
    pub(super) fn method_bits(&mut self, t: P<Type>) -> (bool, bool) {
        let mut parts = Vec::new();
        self.walk_types(t, &mut parts);
        let (mut method, mut function) = (false, false);
        for part in parts {
            if part.symbol().is_none() {
                continue;
            }
            for &signature in self.checker.get_signatures_of_type_exported(part, SignatureKind::Call) {
                if let Some(this) = signature.this_parameter() {
                    if let Some(declaration) = this.value_declaration() {
                        let this_type = self.checker.get_type_at_location(declaration);
                        if this_type.flags().intersects(TypeFlags::Void) {
                            function = true;
                        } else {
                            method = true;
                        }
                    }
                } else if let Some(declaration) = signature.declaration() {
                    if self.is_method_declaration(declaration) {
                        method = true;
                    } else {
                        function = true;
                    }
                }
            }
        }
        (method, function)
    }

    fn walk_types(&mut self, t: P<Type>, out: &mut Vec<P<Type>>) {
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            for &member in t.types() {
                self.walk_types(member, out);
            }
            return;
        }
        match self.checker.get_base_constraint_of_type_exported(t) {
            Some(c) if c != t => self.walk_types(c, out),
            _ => out.push(t),
        }
    }
}
