//! `Remote` over `crucible-oc`'s typed operations in one universe.

use crate::model::{Fields, Kind, ResolvedSettings};
use crate::plan::{Id, Listed};
use crate::provision::{Patch, Remote, Write};
use crucible_oc::Scope;
use crucible_oc::ops::{
    self, BadgeDetails, Details, Image, Sellable, UniverseSettings, badges, passes, products, universes,
};
use crucible_oc::outcome::Failure;
use crucible_oc::transport::Transport;

pub struct Roblox<'t, 'a> {
    pub transport: &'t Transport<'a>,
    pub universe: Id,
}

/// The scopes the calls need that list and read `reads` and send `writes`, as `crucible-oc`'s operations declare them,
/// each once and in order. A public badge read needs none.
pub fn scopes(reads: &[Kind], writes: &[Write]) -> Vec<Scope> {
    let id = Id::MIN;
    let mut scopes = Vec::new();
    for kind in reads {
        match kind {
            Kind::Product => {
                scopes.extend(products::list(id).scopes());
                scopes.extend_from_slice(products::get(id, id).scopes());
            }
            Kind::Pass => {
                scopes.extend(passes::list(id).scopes());
                scopes.extend_from_slice(passes::get(id, id).scopes());
            }
            Kind::Badge => {
                scopes.extend(badges::list_public(id).scopes());
                scopes.extend_from_slice(badges::get_public(id).scopes());
            }
        }
    }
    let details = Details::default();
    for write in writes {
        match *write {
            Write::Create(Kind::Product) => scopes.extend_from_slice(products::create(id, &details).scopes()),
            Write::Create(Kind::Pass) => scopes.extend_from_slice(passes::create(id, &details).scopes()),
            Write::Create(Kind::Badge) => scopes.extend_from_slice(badges::create(id, "", "", true, None).scopes()),
            Write::Update { kind: Kind::Product, .. } => {
                scopes.extend_from_slice(products::update(id, id, &details).scopes());
            }
            Write::Update { kind: Kind::Pass, .. } => {
                scopes.extend_from_slice(passes::update(id, id, &details).scopes())
            }
            Write::Update { kind: Kind::Badge, details, icon } => {
                if details {
                    scopes.extend_from_slice(badges::update(id, &BadgeDetails::default()).scopes());
                }
                if icon {
                    let image = Image { file_name: String::new(), content_type: String::new(), bytes: vec![] };
                    scopes.extend_from_slice(badges::overwrite_icon(id, image).scopes());
                }
            }
            Write::Settings => {
                let settings = UniverseSettings { voice_chat: Some(true), ..UniverseSettings::default() };
                let op = universes::update(id, &settings).expect("a setting is set");
                scopes.extend_from_slice(op.scopes());
            }
        }
    }
    scopes.sort();
    scopes.dedup();
    scopes
}

fn sellable(item: Sellable) -> Listed {
    Listed { id: item.id, name: item.name, description: item.description, price: item.price, on: item.for_sale }
}

fn badge(item: ops::Badge) -> Listed {
    Listed { id: item.id, name: item.name, description: item.description, price: None, on: item.enabled }
}

/// A 404 says Roblox has no such resource. Every other failure stays a failure.
///
/// Roblox reads a badge by its ID whatever universe awards it, so a badge of another universe reads as none, and
/// neither `crucible adopt` nor provision takes it.
fn found<T>(result: Result<T, Failure>) -> Result<Option<T>, Failure> {
    match result {
        Ok(item) => Ok(Some(item)),
        Err(failure) if failure.code == 4 && failure.error["status"] == 404 => Ok(None),
        Err(failure) => Err(failure),
    }
}

impl Remote for Roblox<'_, '_> {
    fn list(&mut self, kind: Kind) -> Result<Vec<Listed>, Failure> {
        let universe = self.universe;
        Ok(match kind {
            Kind::Product => self.transport.list_all(&products::list(universe))?.into_iter().map(sellable).collect(),
            Kind::Pass => self.transport.list_all(&passes::list(universe))?.into_iter().map(sellable).collect(),
            Kind::Badge => self.transport.list_all(&badges::list_public(universe))?.into_iter().map(badge).collect(),
        })
    }

    fn get(&mut self, kind: Kind, id: Id) -> Result<Option<Listed>, Failure> {
        let universe = self.universe;
        Ok(match kind {
            Kind::Product => found(self.transport.run(&products::get(universe, id)))?.map(sellable),
            Kind::Pass => found(self.transport.run(&passes::get(universe, id)))?.map(sellable),
            Kind::Badge => found(self.transport.run(&badges::get_public(id)))?
                .filter(|awarded| awarded.universe == universe)
                .map(|awarded| badge(awarded.badge)),
        })
    }

    fn create(&mut self, kind: Kind, fields: &Fields, icon: Option<&Image>) -> Result<Id, Failure> {
        let universe = self.universe;
        match fields {
            Fields::Priced { name, description, price } => {
                let details = Details {
                    name: Some(name.clone()),
                    description: Some(description.clone()),
                    price: Some(*price),
                    for_sale: Some(true),
                    icon: icon.cloned(),
                };
                let created = match kind {
                    Kind::Product => self.transport.run(&products::create(universe, &details))?,
                    _ => self.transport.run(&passes::create(universe, &details))?,
                };
                Ok(created.id)
            }
            Fields::Badge { name, description, enabled } => {
                Ok(self.transport.run(&badges::create(universe, name, description, *enabled, icon.cloned()))?.id)
            }
        }
    }

    fn update(&mut self, kind: Kind, id: Id, patch: &Patch) -> Result<(), Failure> {
        let universe = self.universe;
        if kind == Kind::Badge {
            let details =
                BadgeDetails { name: patch.name.clone(), description: patch.description.clone(), enabled: patch.on };
            if details != BadgeDetails::default() {
                self.transport.run(&badges::update(id, &details))?;
            }
            if let Some(icon) = &patch.icon {
                self.transport.run(&badges::overwrite_icon(id, icon.clone()))?;
            }
            return Ok(());
        }
        let details = Details {
            name: patch.name.clone(),
            description: patch.description.clone(),
            price: patch.price,
            for_sale: patch.on,
            icon: patch.icon.clone(),
        };
        match kind {
            Kind::Product => self.transport.run(&products::update(universe, id, &details)),
            _ => self.transport.run(&passes::update(universe, id, &details)),
        }
    }

    fn update_settings(&mut self, settings: &ResolvedSettings) -> Result<(), Failure> {
        let settings =
            UniverseSettings { private_server_price: settings.private_server_price, voice_chat: settings.voice_chat };
        match universes::update(self.universe, &settings) {
            Some(op) => self.transport.run(&op),
            None => Ok(()),
        }
    }

    fn read_settings(&mut self) -> Result<ResolvedSettings, Failure> {
        let held = self.transport.run(&universes::get(self.universe))?;
        Ok(ResolvedSettings { private_server_price: held.private_server_price, voice_chat: held.voice_chat })
    }
}
