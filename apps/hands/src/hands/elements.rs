//! An element as the answers name it (its kind and label), whether it is a secret, and finding
//! one again by its ref or its label.

use serde_json::json;

use super::{Hands, object};
use crate::desktop::{Desktop, Link, Text};
use crate::fail::{Fail, Reply, field};
use crate::rules::{Use, is_secret};
use crate::text::{clip, kind};

/// The attributes that label an element, in the order they are tried.
const LABELS: [Text; 4] = [
    Text::Title,
    Text::Description,
    Text::Placeholder,
    Text::Help,
];

impl<D: Desktop> Hands<D> {
    /// The element's role (`AXButton`…), or empty.
    pub(super) fn role(&self, element: &D::Element) -> String {
        self.desktop.text(element, Text::Role).unwrap_or_default()
    }

    /// The element's kind in words ("button", "text field").
    pub(super) fn kind(&self, element: &D::Element) -> String {
        kind(&self.role(element))
    }

    /// The element's label: its title, description, placeholder or help, else the value of the
    /// element that labels it; at most 80 characters.
    pub(super) fn label(&self, element: &D::Element) -> String {
        for attribute in LABELS {
            if let Some(text) = self.desktop.text(element, attribute) {
                return clip(&text, 80);
            }
        }
        self.desktop
            .link(element, Link::TitleElement)
            .and_then(|title| self.desktop.text(&title, Text::Value))
            .map_or_default(|text| clip(&text, 80))
    }

    /// Whether the element is a password field (or a field that wants a code): its value is
    /// never read, set or typed into.
    pub(super) fn secret(&self, element: &D::Element) -> bool {
        let role = self.role(element);
        let subrole = self
            .desktop
            .text(element, Text::Subrole)
            .unwrap_or_default();
        is_secret(&role, &subrole, &self.label(element))
    }

    /// The element `wanted` names in the held app's last snapshot: a ref (`a12`), or an exact
    /// label only one element has.
    pub(super) fn element(&self, wanted: &str) -> Result<D::Element, Fail> {
        let wanted = wanted.trim();
        let refs = self.refs();
        if refs.is_empty() {
            return Err(Fail::bad(
                "No snapshot of this app yet: take mac_app_snapshot first.",
            ));
        }
        if let Some(number) = wanted
            .strip_prefix('a')
            .and_then(|n| n.parse::<usize>().ok())
        {
            return number
                .checked_sub(1)
                .and_then(|i| refs.get(i))
                .cloned()
                .ok_or_else(|| {
                    Fail::bad(format!(
                        "No element {wanted} in this app's last snapshot: take a new one."
                    ))
                });
        }
        let lower = wanted.to_lowercase();
        let same: Vec<(usize, &D::Element)> = refs
            .iter()
            .enumerate()
            .filter(|(_, element)| self.label(element).to_lowercase() == lower)
            .collect();
        match same.as_slice() {
            [(_, element)] => Ok((*element).clone()),
            [] => Err(Fail::bad(format!(
                "No element \"{wanted}\" in this app's last snapshot (a ref like a12, or an exact label)."
            ))),
            many => {
                let named: Vec<String> = many.iter().map(|(i, _)| format!("a{}", i + 1)).collect();
                Err(Fail::bad(format!(
                    "{} elements are \"{wanted}\" ({}): say which by ref.",
                    many.len(),
                    named.join(", ")
                )))
            }
        }
    }

    /// `/describe`: what a ref is, without touching it (the server asks the owner before a
    /// final step by it).
    pub(super) fn describe(&self, body: &Reply) -> Result<Reply, Fail> {
        let target = self.target(body, Use::Look)?;
        let element = self.element(&field(body, "ref"))?;
        Ok(object(json!({
            "kind": self.kind(&element),
            "label": self.label(&element),
            "secret": self.secret(&element),
            "actions": self.desktop.actions(&element),
            "app": target.name,
        })))
    }
}
