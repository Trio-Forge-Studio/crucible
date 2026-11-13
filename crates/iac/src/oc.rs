//! `Remote` over `crucible-oc`'s typed operations in one universe.

use crate::model::{Fields, Kind};
use crate::plan::{Id, Listed};
use crate::provision::{Patch, Remote};
use crucible_oc::ops::{self, BadgeDetails, Details, Image, Sellable, badges, passes, products};
use crucible_oc::outcome::Failure;
use crucible_oc::transport::Transport;

pub struct Roblox<'t, 'a> {
    pub transport: &'t Transport<'a>,
    pub universe: Id,
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
}
