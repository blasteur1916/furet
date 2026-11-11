//! Pertinence des résultats « par le contenu » : correction des « aimants » et seuil.
//!
//! Certains vecteurs (surtout les sons) ressemblent un peu à tout. On mesure ce penchant (le biais)
//! en les comparant à des recherches génériques, et on le retranche du score. Mesuré sur nos fichiers
//! de test (31 recherches, dont 8 sans réponse) : précision 0,84 → 0,97, et meilleur classement.

use crate::serveur::Client;
use anyhow::Result;
use serde_json::Value;

const PHRASES_GENERIQUES: [&str; 40] = [
    "une photo", "un document", "un fichier", "une image", "du texte", "un enregistrement", "une vidéo",
    "quelque chose", "une personne", "un objet", "un paysage", "une liste", "des chiffres", "une page",
    "de la musique", "un son", "une conversation", "un tableau", "un dessin", "un écran", "un animal",
    "une ville", "la nature", "un bâtiment", "un message", "une lettre", "un programme informatique",
    "un graphique", "a picture", "a document", "a recording", "some text", "un logo", "des couleurs",
    "un visage", "de la nourriture", "un véhicule", "un sport", "un vêtement", "un outil",
];
/// Biais moyen mesuré : on le rajoute pour que les scores restent dans une plage familière (0,6 à 0,9).
pub const DECALAGE: f32 = 0.68;
/// Score corrigé minimal pour afficher un résultat (biais moyen + 0,035, un peu en dessous du meilleur F1).
pub const SEUIL: f32 = 0.715;
/// Le biais d'un morceau : moyenne de ses 10 meilleures ressemblances aux phrases génériques.
const MEILLEURES: usize = 10;

pub struct Banque {
    phrases: Vec<Vec<f32>>,
}

impl Banque {
    pub fn charger(modele: &Client) -> Result<Self> {
        let mut phrases = Vec::new();
        for lot in PHRASES_GENERIQUES.chunks(8) {
            let entrees = lot.iter().map(|p| Value::String(format!("task: search result | query: {p}"))).collect();
            phrases.extend(modele.vecteurs(entrees)?);
        }
        Ok(Banque { phrases })
    }

    pub fn biais(&self, v: &[f32]) -> f32 {
        let mut ressemblances: Vec<f32> =
            self.phrases.iter().map(|p| p.iter().zip(v).map(|(a, b)| a * b).sum()).collect();
        ressemblances.sort_by(|a, b| b.total_cmp(a));
        ressemblances[..MEILLEURES].iter().sum::<f32>() / MEILLEURES as f32
    }
}

/// Des mots inventés : une recherche qui leur ressemble beaucoup (« zorglub », « xkqzv ») ne veut sans doute
/// rien dire, et le code, plein de noms bizarres, lui ressemble toujours un peu.
const MOTS_INVENTES: [&str; 12] = [
    "bvnrtq", "plimzork", "gruntfax", "wexvoli", "krazhunf", "dofmipe", "snarglepuff", "trulvex", "yorpik",
    "zantibule", "mubrek", "fliznop",
];
/// Mesuré le 9 novembre 2026 sur l'index de l'utilisateur (19 recherches) : mots inventés seuls 0,83 à 0,87,
/// vraies recherches 0,72 à 0,81. Le seuil monte à partir de 0,80, de 2,5 fois l'excès : les mots inventés
/// n'amènent plus rien (sauf par leurs mots exacts), « logo » ou « azerty » gardent leurs résultats.
const CHARABIA_DEPART: f32 = 0.80;
const CHARABIA_PENTE: f32 = 2.5;

/// Le vecteur moyen des mots inventés, pour adapter le seuil à chaque recherche.
pub struct Charabia {
    moyen: Vec<f32>,
}

impl Charabia {
    pub fn charger(modele: &Client) -> Result<Self> {
        let mut moyen: Vec<f32> = Vec::new();
        for m in MOTS_INVENTES {
            let v = modele.vecteur_requete(m)?;
            if moyen.is_empty() {
                moyen = vec![0.0; v.len()];
            }
            moyen.iter_mut().zip(v).for_each(|(a, b)| *a += b);
        }
        let n = moyen.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        moyen.iter_mut().for_each(|x| *x /= n);
        Ok(Charabia { moyen })
    }

    /// Le seuil pour la recherche dont `vecteur` est le vecteur.
    pub fn seuil(&self, vecteur: &[f32]) -> f32 {
        let ressemblance: f32 = self.moyen.iter().zip(vecteur).map(|(a, b)| a * b).sum();
        SEUIL + CHARABIA_PENTE * (ressemblance - CHARABIA_DEPART).max(0.0)
    }
}

/// Score affiché : ressemblance moins le biais du morceau, recentré.
pub fn score(ressemblance: f32, biais: Option<f32>) -> f32 {
    ressemblance - biais.unwrap_or(DECALAGE) + DECALAGE
}
