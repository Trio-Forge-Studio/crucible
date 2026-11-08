//! The ABI between Crucible's transform and its runtime, as one table: each function of the runtime's emit entry that
//! compiled code calls and what the compiler passes it, the class registration record, the `n` schema builders, the
//! `t` attribute checks, and the subpaths a game may import. `cargo xtask abi` writes the runtime's
//! `src/emit/abi.d.ts` from this table, and the runtime's emit entry must satisfy that declaration.

mod declaration;

pub use declaration::declaration;

/// Raised by any change a compiled game would notice: a function, parameter, record field, schema builder or check
/// that is added, removed, renamed, moved or retyped.
pub const VERSION: u32 = 1;

/// The field of the runtime's `package.json` that records the ABI version the runtime implements.
pub const MANIFEST_FIELD: &str = "crucibleAbi";

/// The version a runtime implements when its `package.json` has no `crucibleAbi` field. Every release before the field,
/// up to 0.4.0, implements version 1.
pub const UNDECLARED: u32 = 1;

pub const PACKAGE: &str = "@trioforge/crucible";

/// The `exports` key of the entry compiled code calls.
pub const EMIT: &str = "./emit";

/// The subpaths of the runtime a game may import besides its root. CRU004 rejects every other one.
pub const SUBPATHS: &[&str] = &["server", "client", "util/random", "maths/spline"];

/// A function compiled code calls, with the parameters the compiler passes in order.
#[derive(Clone, Copy, Debug)]
pub struct Function {
    pub name: &'static str,
    pub parameters: &'static [Parameter],
}

#[derive(Clone, Copy, Debug)]
pub struct Parameter {
    pub name: &'static str,
    /// The TypeScript type of what the compiler passes, as `abi.d.ts` spells it. A rest parameter's type is one
    /// element's.
    pub ty: &'static str,
    pub arity: Arity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arity {
    Required,
    /// The compiler may leave it out.
    Optional,
    /// Any number of values, which ends the list.
    Rest,
}

const fn required(name: &'static str, ty: &'static str) -> Parameter {
    Parameter { name, ty, arity: Arity::Required }
}

const fn optional(name: &'static str, ty: &'static str) -> Parameter {
    Parameter { name, ty, arity: Arity::Optional }
}

const fn rest(name: &'static str, ty: &'static str) -> Parameter {
    Parameter { name, ty, arity: Arity::Rest }
}

/// A value the compiler passes on without building it, such as a game's own value or what another call of this table
/// returned. `abi.d.ts` declares it `never`, so the check covers its position and not its type.
const PASSED: &str = "Passed";
/// The check the runtime calls with a behaviour attribute's value: each `t` value, and what each `t` function returns.
const ATTRIBUTE_CHECK: &str = "Check";
/// A wire schema node: each `n` value, and what each `n` function returns. Compiled code only passes it on, so
/// `abi.d.ts` declares it `object`, which keeps it a table and not a primitive.
const SCHEMA_NODE: &str = "Schema";
const STRING: &str = "string";
const LITERAL: &str = "Literal";
const CLASS_NAMES: &str = "readonly (keyof Instances)[]";

pub const DEFINE_CLASS: Function =
    Function { name: "defineClass", parameters: &[required("klass", PASSED), required("record", "ClassRecord")] };
pub const RESOLVE: Function = Function { name: "resolve", parameters: &[required("id", STRING)] };
pub const RESOLVE_ALL: Function = Function { name: "resolveAll", parameters: &[required("id", STRING)] };
/// Called with the behaviour, the field's name and the macro's own arguments.
pub const ATTRIBUTE: Function = Function {
    name: "attribute",
    parameters: &[required("object", PASSED), required("field", STRING), optional("fallback", PASSED)],
};
/// Called with the subsystem, the field's name and the macro's own arguments.
pub const RECEIVE: Function = Function {
    name: "receive",
    parameters: &[
        required("owner", PASSED),
        required("field", STRING),
        required("target", PASSED),
        required("callback", PASSED),
    ],
};

// A declaration macro's call passes its name, the arguments the game wrote after the name, its wire schemas and then
// its tail.
pub const TO_SERVER: Function =
    Function { name: "toServer", parameters: &[required("name", STRING), required("payload", PASSED)] };
pub const TO_CLIENT: Function =
    Function { name: "toClient", parameters: &[required("name", STRING), required("payload", PASSED)] };
pub const ASK_SERVER: Function = Function {
    name: "askServer",
    parameters: &[
        required("name", STRING),
        required("request", PASSED),
        required("response", PASSED),
        required("timeout", "number | undefined"),
    ],
};
pub const REPLICATED: Function = Function {
    name: "replicated",
    parameters: &[required("name", STRING), required("initial", PASSED), required("value", PASSED)],
};
pub const REPLICATED_PER_PLAYER: Function = Function {
    name: "replicatedPerPlayer",
    parameters: &[required("name", STRING), required("initial", PASSED), required("value", PASSED)],
};
pub const DATA_STORE: Function = Function {
    name: "dataStore",
    parameters: &[required("name", STRING), required("options", PASSED), required("value", PASSED)],
};
pub const PROFILE_STORE: Function = Function {
    name: "profileStore",
    parameters: &[
        required("name", STRING),
        required("options", PASSED),
        required("persist", PASSED),
        required("replica", PASSED),
        required("serverOnlyPaths", "readonly (readonly string[])[]"),
    ],
};
pub const PROFILE_MESSAGE: Function =
    Function { name: "profileMessage", parameters: &[required("name", STRING), required("payload", PASSED)] };
pub const CONFIG: Function = Function {
    name: "config",
    parameters: &[required("name", STRING), required("fallback", PASSED), required("value", PASSED)],
};
pub const PLAYER_CONFIG: Function = Function {
    name: "playerConfig",
    parameters: &[required("name", STRING), required("fallback", PASSED), required("value", PASSED)],
};

pub const FUNCTIONS: &[Function] = &[
    DEFINE_CLASS,
    RESOLVE,
    RESOLVE_ALL,
    ATTRIBUTE,
    RECEIVE,
    TO_SERVER,
    TO_CLIENT,
    ASK_SERVER,
    REPLICATED,
    REPLICATED_PER_PLAYER,
    DATA_STORE,
    PROFILE_STORE,
    PROFILE_MESSAGE,
    CONFIG,
    PLAYER_CONFIG,
];

/// An export whose members compiled code reads, such as `n.u8`, or calls, such as `n.array(element)`.
#[derive(Clone, Copy, Debug)]
pub struct Namespace {
    pub name: &'static str,
    /// The type of each value, and of what each function returns, as `abi.d.ts` spells it.
    pub ty: &'static str,
    pub values: &'static [&'static str],
    pub functions: &'static [Function],
}

/// The builders of a declaration's wire schema.
pub const N: Namespace = Namespace {
    name: "n",
    ty: SCHEMA_NODE,
    values: &["nil", "bool", "f64", "f32", "u8", "u16", "u32", "string", "dynamic", "vector3", "color3", "cframe"],
    functions: &[
        Function { name: "literal", parameters: &[required("value", LITERAL)] },
        Function { name: "instance", parameters: &[required("classes", CLASS_NAMES)] },
        Function { name: "object", parameters: &[required("fields", "readonly (readonly [string, Passed])[]")] },
        Function { name: "array", parameters: &[required("element", PASSED)] },
        Function { name: "tuple", parameters: &[required("elements", "readonly Passed[]")] },
        Function { name: "map", parameters: &[required("key", PASSED), required("value", PASSED)] },
        Function { name: "union", parameters: &[required("branches", "readonly Passed[]")] },
        Function {
            name: "variant",
            parameters: &[required("key", STRING), required("value", LITERAL), required("body", PASSED)],
        },
    ],
};

/// The checks a behaviour attribute's value must pass.
pub const T: Namespace = Namespace {
    name: "t",
    ty: ATTRIBUTE_CHECK,
    values: &["string", "number", "boolean", "vector3", "color3"],
    functions: &[
        Function { name: "literal", parameters: &[required("expected", LITERAL)] },
        Function { name: "optional", parameters: &[required("check", ATTRIBUTE_CHECK)] },
        Function { name: "union", parameters: &[rest("checks", ATTRIBUTE_CHECK)] },
    ],
};

pub const NAMESPACES: &[Namespace] = &[N, T];

/// A field of a table the compiler builds.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub name: &'static str,
    pub ty: &'static str,
    /// The compiler leaves it out when it is empty.
    pub optional: bool,
}

const fn field(name: &'static str, ty: &'static str) -> Field {
    Field { name, ty, optional: false }
}

/// The record `defineClass` registers a class with. Its `kind` picks a subsystem's fields or a behaviour's.
pub mod record {
    use super::{ATTRIBUTE_CHECK, CLASS_NAMES, Field, STRING, field};

    pub const KIND: &str = "kind";
    pub const SUBSYSTEM: &str = "subsystem";
    pub const BEHAVIOUR: &str = "behaviour";

    pub const ID: &str = "id";
    /// The constructor's dependencies in parameter order. Each is a type id, or `{ all = id }` for every class that
    /// provides one.
    pub const DEPS: &str = "deps";
    pub const ALL: &str = "all";
    pub const PROVIDES: &str = "provides";
    pub const TAG: &str = "tag";
    pub const INSTANCE: &str = "instance";
    /// The behaviour's attributes by field name.
    pub const ATTRIBUTES: &str = "attributes";

    pub const NAME: &str = "name";
    pub const CHECK: &str = "check";
    pub const REQUIRED: &str = "required";

    /// Every record's fields after `kind`.
    pub const CLASS: &[Field] =
        &[field(ID, STRING), field(DEPS, "readonly Dependency[]"), field(PROVIDES, "readonly string[]")];
    /// What a behaviour's record adds.
    pub const BEHAVIOUR_FIELDS: &[Field] = &[
        field(TAG, STRING),
        field(INSTANCE, CLASS_NAMES),
        Field { name: ATTRIBUTES, ty: "{ readonly [field: string]: AttributeRecord }", optional: true },
    ];
    /// An entry of `attributes`.
    pub const ATTRIBUTE: &[Field] = &[field(NAME, STRING), field(CHECK, ATTRIBUTE_CHECK), field(REQUIRED, "boolean")];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_where_compiled_code_reads_them() {
        let mut top: Vec<&str> = FUNCTIONS.iter().map(|f| f.name).chain(NAMESPACES.iter().map(|n| n.name)).collect();
        let count = top.len();
        top.sort_unstable();
        top.dedup();
        assert_eq!(top.len(), count, "a top-level name repeats");
        for namespace in NAMESPACES {
            let mut members: Vec<&str> =
                namespace.values.iter().copied().chain(namespace.functions.iter().map(|f| f.name)).collect();
            let count = members.len();
            members.sort_unstable();
            members.dedup();
            assert_eq!(members.len(), count, "a member of {} repeats", namespace.name);
        }
    }

    #[test]
    fn a_rest_parameter_ends_its_list_and_an_optional_one_follows_only_optional_ones() {
        for function in FUNCTIONS.iter().chain(NAMESPACES.iter().flat_map(|n| n.functions)) {
            let arities: Vec<Arity> = function.parameters.iter().map(|p| p.arity).collect();
            if let Some(at) = arities.iter().position(|a| *a == Arity::Rest) {
                assert_eq!(at + 1, arities.len(), "{} has a parameter after its rest parameter", function.name);
            }
            if let Some(at) = arities.iter().position(|a| *a == Arity::Optional) {
                assert!(
                    arities[at..].iter().all(|a| *a != Arity::Required),
                    "{} has a required parameter after an optional one",
                    function.name
                );
            }
        }
    }
}
