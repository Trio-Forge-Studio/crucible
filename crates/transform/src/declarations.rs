use crucible_abi::Function;

pub const MESSAGE_NAME_MAX_BYTES: usize = 64;
pub const MAX_ASK_TIMEOUT_SECONDS: f64 = 120.0;
pub const MAX_UNION_BRANCHES: usize = 256;
pub const MAX_SCHEMA_NODES: usize = 4096;
pub const MAX_DEPTH: u32 = 32;
pub const SYNC_VALUE_DEPTH: u32 = 28;
pub const STORE_NAME_MAX_BYTES: usize = 50;
pub const CONFIG_NAME_MAX_BYTES: usize = 64;
pub const PERSIST_DEPTH: u32 = 28;
pub const VIEW_NAME_PREFIX: &str = "Crucible.Data.";
pub const VIEW_NAME_MAX_BYTES: usize = 64;
pub const RESERVED_HANDLE_NAMES: [&str; 4] = ["set", "update", "insert", "remove"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Server,
    Client,
    Shared,
}

impl Side {
    pub const fn folder(self) -> &'static str {
        match self {
            Side::Server => "server",
            Side::Client => "client",
            Side::Shared => "shared",
        }
    }
}

/// A macro that declares a message, replicated value, store, profile message or config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeclarationKind {
    ToServer,
    ToClient,
    AskServer,
    Replicated,
    ReplicatedPerPlayer,
    ProfileStore,
    DataStore,
    ProfileMessage,
    Config,
    PlayerConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Payload,
    State,
    Persist,
}

impl Position {
    pub const fn depth_limit(self) -> u32 {
        match self {
            Position::Payload => MAX_DEPTH,
            Position::State => SYNC_VALUE_DEPTH,
            Position::Persist => PERSIST_DEPTH,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeArguments {
    Exactly(usize),
    /// The value type may be inferred from the call's arguments.
    AtMostOne,
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub label: &'static str,
    pub side: Side,
    pub names: &'static str,
    pub max_bytes: usize,
    pub type_arguments: TypeArguments,
    pub arguments: usize,
    pub passed: &'static [usize],
    pub position: Position,
    pub store: bool,
    pub handles: bool,
}

const MESSAGE: Spec = Spec {
    label: "messages",
    side: Side::Shared,
    names: "Message",
    max_bytes: MESSAGE_NAME_MAX_BYTES,
    type_arguments: TypeArguments::Exactly(1),
    arguments: 1,
    passed: &[],
    position: Position::Payload,
    store: false,
    handles: false,
};

const REPLICATED: Spec = Spec {
    label: "replicated state",
    names: "Replicated",
    arguments: 2,
    passed: &[1],
    position: Position::State,
    ..MESSAGE
};

const CONFIG: Spec = Spec {
    label: "configs",
    side: Side::Server,
    names: "Config",
    max_bytes: CONFIG_NAME_MAX_BYTES,
    position: Position::Persist,
    ..REPLICATED
};

impl DeclarationKind {
    pub const ALL: [DeclarationKind; 10] = [
        DeclarationKind::ToServer,
        DeclarationKind::ToClient,
        DeclarationKind::AskServer,
        DeclarationKind::Replicated,
        DeclarationKind::ReplicatedPerPlayer,
        DeclarationKind::ProfileStore,
        DeclarationKind::DataStore,
        DeclarationKind::ProfileMessage,
        DeclarationKind::Config,
        DeclarationKind::PlayerConfig,
    ];

    pub const fn spec(self) -> Spec {
        match self {
            DeclarationKind::ToServer | DeclarationKind::ToClient => MESSAGE,
            DeclarationKind::AskServer => Spec { type_arguments: TypeArguments::Exactly(2), arguments: 2, ..MESSAGE },
            DeclarationKind::Replicated | DeclarationKind::ReplicatedPerPlayer => REPLICATED,
            DeclarationKind::ProfileStore => Spec {
                label: "profile stores",
                names: "Store",
                max_bytes: STORE_NAME_MAX_BYTES,
                type_arguments: TypeArguments::AtMostOne,
                position: Position::Persist,
                store: true,
                handles: true,
                ..REPLICATED
            },
            DeclarationKind::DataStore => {
                Spec { label: "data stores", names: "Store", max_bytes: STORE_NAME_MAX_BYTES, store: true, ..CONFIG }
            }
            DeclarationKind::ProfileMessage => Spec {
                label: "profile messages",
                side: Side::Server,
                names: "Profile message",
                position: Position::Persist,
                ..MESSAGE
            },
            DeclarationKind::Config | DeclarationKind::PlayerConfig => CONFIG,
        }
    }

    pub const fn function(self) -> Function {
        match self {
            DeclarationKind::ToServer => crucible_abi::TO_SERVER,
            DeclarationKind::ToClient => crucible_abi::TO_CLIENT,
            DeclarationKind::AskServer => crucible_abi::ASK_SERVER,
            DeclarationKind::Replicated => crucible_abi::REPLICATED,
            DeclarationKind::ReplicatedPerPlayer => crucible_abi::REPLICATED_PER_PLAYER,
            DeclarationKind::ProfileStore => crucible_abi::PROFILE_STORE,
            DeclarationKind::DataStore => crucible_abi::DATA_STORE,
            DeclarationKind::ProfileMessage => crucible_abi::PROFILE_MESSAGE,
            DeclarationKind::Config => crucible_abi::CONFIG,
            DeclarationKind::PlayerConfig => crucible_abi::PLAYER_CONFIG,
        }
    }

    pub const fn name(self) -> &'static str {
        self.function().name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_limits(file: &str) -> Vec<(String, String)> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/crucible/src/internal").join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} exists", path.display()));
        text.lines()
            .filter_map(|line| line.trim().strip_prefix("export const "))
            .filter_map(|rest| {
                let (name, value) = rest.split_once('=')?;
                let value = value.trim().trim_end_matches(';').trim();
                Some((name.split(':').next()?.trim().to_owned(), value.trim_matches('"').to_owned()))
            })
            .collect()
    }

    fn table() -> Vec<(&'static str, &'static str, String)> {
        vec![
            ("network/limits.ts", "MESSAGE_NAME_MAX_BYTES", MESSAGE_NAME_MAX_BYTES.to_string()),
            ("network/limits.ts", "MAX_ASK_TIMEOUT_SECONDS", MAX_ASK_TIMEOUT_SECONDS.to_string()),
            ("network/limits.ts", "MAX_UNION_BRANCHES", MAX_UNION_BRANCHES.to_string()),
            ("network/limits.ts", "MAX_SCHEMA_NODES", MAX_SCHEMA_NODES.to_string()),
            ("network/limits.ts", "MAX_DEPTH", MAX_DEPTH.to_string()),
            ("reactive/limits.ts", "MESSAGE_NAME_MAX_BYTES", MESSAGE_NAME_MAX_BYTES.to_string()),
            ("reactive/limits.ts", "SYNC_VALUE_DEPTH", SYNC_VALUE_DEPTH.to_string()),
            ("data/limits.ts", "STORE_NAME_MAX_BYTES", STORE_NAME_MAX_BYTES.to_string()),
            ("data/limits.ts", "MESSAGE_NAME_MAX_BYTES", MESSAGE_NAME_MAX_BYTES.to_string()),
            ("data/limits.ts", "PERSIST_DEPTH", PERSIST_DEPTH.to_string()),
            ("data/limits.ts", "VIEW_NAME_PREFIX", VIEW_NAME_PREFIX.to_owned()),
            ("data/limits.ts", "VIEW_NAME_MAX_BYTES", VIEW_NAME_MAX_BYTES.to_string()),
            ("configs/limits.ts", "CONFIG_NAME_MAX_BYTES", CONFIG_NAME_MAX_BYTES.to_string()),
        ]
    }

    #[test]
    fn limits_equal_the_runtime_limits() {
        let mut compared = Vec::new();
        for (file, name, expected) in table() {
            if let Some((_, value)) = runtime_limits(file).into_iter().find(|(n, _)| n == name) {
                assert_eq!(value, expected, "{file} {name}");
                compared.push(name);
            }
        }
        for name in ["MAX_DEPTH", "MAX_SCHEMA_NODES", "SYNC_VALUE_DEPTH", "STORE_NAME_MAX_BYTES", "PERSIST_DEPTH"] {
            assert!(compared.contains(&name), "the runtime declares {name}");
        }
        assert!(compared.contains(&"CONFIG_NAME_MAX_BYTES"));
    }

    #[test]
    fn every_kind_has_its_own_emit_function() {
        let names: std::collections::HashSet<&str> = DeclarationKind::ALL.iter().map(|k| k.name()).collect();
        assert_eq!(names.len(), DeclarationKind::ALL.len());
        for kind in DeclarationKind::ALL {
            assert!(crucible_abi::FUNCTIONS.iter().any(|f| f.name == kind.name()), "{kind:?}");
        }
    }
}
