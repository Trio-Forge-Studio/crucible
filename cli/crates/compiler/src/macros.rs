//! The macros the lowering expands inline. roblox-ts 3.0.0's call, constructor, identifier and property-call tables,
//! plus Crucible's `attribute`, `inject` and `boot`. The extractor matches each by symbol identity, never by name alone.

/// How a macro is found in source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MacroKind {
    /// A call to a global function, such as `typeIs(x, "number")`.
    Call,
    /// `new` on a global whose type is the named constructor interface, such as `new Map()`.
    Constructor,
    /// A reference to a global value, such as `Promise`.
    Identifier,
    /// A call to a method declared on the named interface, such as `array.push(x)`.
    Method,
    /// A call to a function that `@trioforge/crucible` or one of its side entries exports.
    Crucible,
}

macro_rules! macros {
    (@owner) => {
        None
    };
    (@owner $owner:literal) => {
        Some($owner)
    };
    ($($variant:ident = $kind:ident $(($owner:literal))? $name:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Macro {
            $($variant,)*
        }

        impl Macro {
            pub const ALL: &'static [Macro] = &[$(Macro::$variant,)*];

            pub const fn kind(self) -> MacroKind {
                match self {
                    $(Macro::$variant => MacroKind::$kind,)*
                }
            }

            /// The interface that declares a method macro.
            pub const fn owner(self) -> Option<&'static str> {
                match self {
                    $(Macro::$variant => macros!(@owner $($owner)?),)*
                }
            }

            /// The function, constructor interface, global, method or export name.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Macro::$variant => $name,)*
                }
            }
        }
    };
}

macros! {
    Assert = Call "assert",
    TypeOf = Call "typeOf",
    TypeIs = Call "typeIs",
    ClassIs = Call "classIs",
    Identity = Call "identity",
    Range = Call "$range",
    Tuple = Call "$tuple",
    GetModuleTree = Call "$getModuleTree",

    NewArray = Constructor "ArrayConstructor",
    NewSet = Constructor "SetConstructor",
    NewMap = Constructor "MapConstructor",
    NewWeakSet = Constructor "WeakSetConstructor",
    NewWeakMap = Constructor "WeakMapConstructor",
    NewReadonlyMap = Constructor "ReadonlyMapConstructor",
    NewReadonlySet = Constructor "ReadonlySetConstructor",

    Promise = Identifier "Promise",

    CFrameAdd = Method("CFrame") "add",
    CFrameSub = Method("CFrame") "sub",
    CFrameMul = Method("CFrame") "mul",
    UDimAdd = Method("UDim") "add",
    UDimSub = Method("UDim") "sub",
    UDim2Add = Method("UDim2") "add",
    UDim2Sub = Method("UDim2") "sub",
    Vector2Add = Method("Vector2") "add",
    Vector2Sub = Method("Vector2") "sub",
    Vector2Mul = Method("Vector2") "mul",
    Vector2Div = Method("Vector2") "div",
    Vector2Idiv = Method("Vector2") "idiv",
    Vector2int16Add = Method("Vector2int16") "add",
    Vector2int16Sub = Method("Vector2int16") "sub",
    Vector2int16Mul = Method("Vector2int16") "mul",
    Vector2int16Div = Method("Vector2int16") "div",
    Vector3Add = Method("Vector3") "add",
    Vector3Sub = Method("Vector3") "sub",
    Vector3Mul = Method("Vector3") "mul",
    Vector3Div = Method("Vector3") "div",
    Vector3Idiv = Method("Vector3") "idiv",
    Vector3int16Add = Method("Vector3int16") "add",
    Vector3int16Sub = Method("Vector3int16") "sub",
    Vector3int16Mul = Method("Vector3int16") "mul",
    Vector3int16Div = Method("Vector3int16") "div",
    NumberIdiv = Method("Number") "idiv",
    StringSize = Method("String") "size",
    StringByte = Method("String") "byte",
    StringFind = Method("String") "find",
    StringFormat = Method("String") "format",
    StringGmatch = Method("String") "gmatch",
    StringGsub = Method("String") "gsub",
    StringLower = Method("String") "lower",
    StringMatch = Method("String") "match",
    StringRep = Method("String") "rep",
    StringReverse = Method("String") "reverse",
    StringSplit = Method("String") "split",
    StringSub = Method("String") "sub",
    StringUpper = Method("String") "upper",
    ArrayLikeSize = Method("ArrayLike") "size",
    ReadonlyArrayIsEmpty = Method("ReadonlyArray") "isEmpty",
    ReadonlyArrayJoin = Method("ReadonlyArray") "join",
    ReadonlyArrayMove = Method("ReadonlyArray") "move",
    ReadonlyArrayIncludes = Method("ReadonlyArray") "includes",
    ReadonlyArrayIndexOf = Method("ReadonlyArray") "indexOf",
    ReadonlyArrayEvery = Method("ReadonlyArray") "every",
    ReadonlyArraySome = Method("ReadonlyArray") "some",
    ReadonlyArrayForEach = Method("ReadonlyArray") "forEach",
    ReadonlyArrayMap = Method("ReadonlyArray") "map",
    ReadonlyArrayMapFiltered = Method("ReadonlyArray") "mapFiltered",
    ReadonlyArrayFilterUndefined = Method("ReadonlyArray") "filterUndefined",
    ReadonlyArrayFilter = Method("ReadonlyArray") "filter",
    ReadonlyArrayReduce = Method("ReadonlyArray") "reduce",
    ReadonlyArrayFind = Method("ReadonlyArray") "find",
    ReadonlyArrayFindIndex = Method("ReadonlyArray") "findIndex",
    ArrayPush = Method("Array") "push",
    ArrayPop = Method("Array") "pop",
    ArrayShift = Method("Array") "shift",
    ArrayUnshift = Method("Array") "unshift",
    ArrayInsert = Method("Array") "insert",
    ArrayRemove = Method("Array") "remove",
    ArrayUnorderedRemove = Method("Array") "unorderedRemove",
    ArraySort = Method("Array") "sort",
    ArrayClear = Method("Array") "clear",
    ReadonlySetIsEmpty = Method("ReadonlySet") "isEmpty",
    ReadonlySetSize = Method("ReadonlySet") "size",
    ReadonlySetHas = Method("ReadonlySet") "has",
    ReadonlySetForEach = Method("ReadonlySet") "forEach",
    SetDelete = Method("Set") "delete",
    SetClear = Method("Set") "clear",
    SetAdd = Method("Set") "add",
    ReadonlyMapIsEmpty = Method("ReadonlyMap") "isEmpty",
    ReadonlyMapSize = Method("ReadonlyMap") "size",
    ReadonlyMapHas = Method("ReadonlyMap") "has",
    ReadonlyMapForEach = Method("ReadonlyMap") "forEach",
    ReadonlyMapGet = Method("ReadonlyMap") "get",
    MapDelete = Method("Map") "delete",
    MapClear = Method("Map") "clear",
    MapSet = Method("Map") "set",
    PromiseThen = Method("Promise") "then",

    Attribute = Crucible "attribute",
    Inject = Crucible "inject",
    Boot = Crucible "boot",
}
