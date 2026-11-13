use crucible_iac::journal::{self, Journal};
use crucible_iac::lock::{self, Lock};
use crucible_iac::model::{Experience, Problem};
use crucible_iac::plan::{Action, Inputs, Observed, Plan, plan, undeclared};
use crucible_project::Manifest;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// The environment in crucible.toml. Without one, plan every environment.
    pub env: Option<String>,
    /// Print one JSON document instead of text.
    #[arg(long)]
    pub json: bool,
}

pub(crate) struct Failure {
    pub code: u8,
    pub message: String,
    pub problems: Vec<Problem>,
}

impl Failure {
    pub fn usage(message: impl Into<String>) -> Self {
        Self { code: 2, message: message.into(), problems: vec![] }
    }

    pub fn read(path: &Path, error: &std::io::Error) -> Self {
        Self { code: 1, message: format!("cannot read {}: {error}", path.display()), problems: vec![] }
    }
}

pub fn main(args: Args) -> ExitCode {
    let result = std::env::current_dir()
        .map_err(|error| Failure::read(Path::new("."), &error))
        .and_then(|root| run(&root, args.env.as_deref()));
    let written = match &result {
        Ok(plans) if args.json => print(&json!({ "environments": plans }).to_string()),
        Ok(plans) => print(&render(plans)),
        Err(failure) if args.json => print(
            &json!({ "error": {
                "kind": if failure.code == 2 { "usage" } else { "io" },
                "message": failure.message,
                "problems": failure.problems,
            }})
            .to_string(),
        ),
        Err(failure) => {
            let mut text = failure.message.clone();
            for problem in &failure.problems {
                text += &format!("\n  {problem}");
            }
            eprintln!("{text}");
            Ok(())
        }
    };
    match (result, written) {
        (Err(failure), _) => ExitCode::from(failure.code),
        (Ok(_), Err(_)) => ExitCode::from(1),
        (Ok(_), Ok(())) => ExitCode::SUCCESS,
    }
}

fn print(text: &str) -> std::io::Result<()> {
    writeln!(std::io::stdout().lock(), "{text}")
}

/// The files a plan reads.
pub(crate) struct Project {
    pub manifest: Manifest,
    pub experience: Experience,
    pub lock: Option<Lock>,
    pub journals: BTreeMap<String, Journal>,
}

impl Project {
    pub fn load(root: &Path, command: &str) -> Result<Self, Failure> {
        let read = |path: &Path| match std::fs::read_to_string(root.join(path)) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Failure::read(path, &error)),
        };
        let text = read(Path::new(crucible_project::FILE))?.ok_or_else(|| {
            Failure::usage(format!("{command} needs crucible.toml. Run `crucible docs experience` for an example"))
        })?;
        let manifest = Manifest::parse(&text).map_err(|error| Failure::usage(error.to_string()))?;
        let experience = Experience::new(&manifest).map_err(|problems| Failure {
            code: 2,
            message: "crucible.toml breaks the experience rules:".into(),
            problems,
        })?;
        let lock = read(Path::new(lock::FILE))?
            .map(|text| Lock::parse(&text))
            .transpose()
            .map_err(|error| Failure::usage(error.to_string()))?;
        let journals = journals(root)?;
        Ok(Self { manifest, experience, lock, journals })
    }

    pub fn unknown(&self, name: &str) -> Failure {
        Failure::usage(format!(
            "unknown environment {name}. Environments: {}",
            self.manifest.envs.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    }

    /// The inputs of `env`'s plan, with what Roblox holds when provision has listed it.
    pub fn inputs<'a>(&'a self, env: &'a str, observed: Option<&'a Observed>) -> Inputs<'a> {
        Inputs {
            experience: &self.experience,
            env_name: env,
            env: &self.manifest.envs[env],
            lock: self.lock.as_ref().and_then(|lock| lock.env(env)),
            journal: self.journals.get(env),
            observed,
        }
    }
}

fn run(root: &Path, only: Option<&str>) -> Result<Vec<Plan>, Failure> {
    let project = Project::load(root, "plan")?;
    let Project { manifest, lock, journals, .. } = &project;
    let names: Vec<&String> = match only {
        Some(name) => {
            vec![manifest.envs.get_key_value(name).map(|(name, _)| name).ok_or_else(|| project.unknown(name))?]
        }
        None => manifest.envs.keys().collect(),
    };
    if names.is_empty() {
        return Err(Failure::usage("crucible.toml configures no environment. Add one with [env.<name>]"));
    }
    let mut plans = Vec::new();
    let mut problems = undeclared(&manifest.envs, lock.as_ref(), journals);
    for name in names {
        match plan(&project.inputs(name, None), &mut |path| std::fs::read(root.join(path))) {
            Ok(plan) => plans.push(plan),
            Err(found) => problems.extend(found),
        }
    }
    if problems.is_empty() {
        Ok(plans)
    } else {
        Err(Failure { code: 2, message: "the plan breaks the provisioning rules:".into(), problems })
    }
}

/// Every `<env>.json` in the journal folder, by the environment its name gives, declared or not.
fn journals(root: &Path) -> Result<BTreeMap<String, Journal>, Failure> {
    let folder = journal::path("env");
    let folder = folder.parent().expect("a journal lives in a folder");
    let entries = match std::fs::read_dir(root.join(folder)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(Failure::read(folder, &error)),
    };
    let mut journals = BTreeMap::new();
    for entry in entries {
        let path = entry.map_err(|error| Failure::read(folder, &error))?.path();
        let Some(env) = path.extension().filter(|extension| *extension == "json").and(path.file_stem()) else {
            continue;
        };
        let env = env.to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).map_err(|error| Failure::read(&journal::path(&env), &error))?;
        let journal = Journal::parse(&env, &text).map_err(|error| {
            Failure::usage(format!(
                "{error}. Each .json file in {} is the journal of the environment its name gives, so a file that is \
                 not one stops every plan. If it is not, move it out of the folder",
                folder.display()
            ))
        })?;
        journals.insert(env, journal);
    }
    Ok(journals)
}

pub(crate) fn render(plans: &[Plan]) -> String {
    let mut text = String::new();
    for plan in plans {
        text += &format!("{} (universe {})\n", plan.environment, plan.universe);
        let rows: Vec<[String; 5]> = plan
            .steps
            .iter()
            .map(|step| {
                let mut detail: Vec<String> = step.changes.iter().map(|change| label(&json!(change))).collect();
                if let Some(old) = &step.moved_from {
                    detail.push(format!("from {old}"));
                }
                if let Some(intent) = &step.intent {
                    let resolved = if intent.resolved { ", resolved by the lock" } else { "" };
                    detail.push(format!("intent sent {:?}{resolved}", intent.name));
                }
                detail.extend(step.note.clone());
                detail.extend(step.settings.iter().map(|setting| format!("{} = {}", setting.key, setting.value)));
                [
                    label(&json!(step.action)),
                    format!("{} {}", step.kind, step.key),
                    step.handle.clone().unwrap_or_default(),
                    step.id.clone().unwrap_or_default(),
                    detail.join(", "),
                ]
            })
            .collect();
        let widths: Vec<usize> = (0..4).map(|at| rows.iter().map(|row| row[at].len()).max().unwrap_or(0)).collect();
        for row in rows {
            let line = format!(
                "  {:w0$}  {:w1$}  {:w2$}  {:w3$}  {}",
                row[0],
                row[1],
                row[2],
                row[3],
                row[4],
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2],
                w3 = widths[3]
            );
            text += line.trim_end();
            text.push('\n');
        }
        let counts: Vec<String> =
            plan.summary.iter().map(|(action, count)| format!("{count} {}", label(&json!(action)))).collect();
        text += &format!("  {}.\n", if counts.is_empty() { "Nothing declared".into() } else { counts.join(", ") });
        for warning in &plan.warnings {
            text += &format!("  Warning: {warning}\n");
        }
        if plan.summary.contains_key(&Action::Create) {
            text += "  Provision adopts the one existing resource with a create's name instead of creating it.\n";
        }
        text.push('\n');
    }
    text.trim_end().to_owned()
}

fn label(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}
