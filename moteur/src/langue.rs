//! La langue des textes que Furet montre : anglais (par défaut) ou français. L'appli la choisit au lancement
//! (réglage, sinon la langue du système). Portable.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Langue {
    Anglais,
    Francais,
}

static FRANCAIS: AtomicBool = AtomicBool::new(false);

pub fn definir(langue: Langue) {
    FRANCAIS.store(langue == Langue::Francais, Ordering::Relaxed);
}

pub fn francais() -> bool {
    FRANCAIS.load(Ordering::Relaxed)
}

/// Le texte dans la langue choisie.
pub fn t<'a>(fr: &'a str, en: &'a str) -> &'a str {
    if francais() { fr } else { en }
}
