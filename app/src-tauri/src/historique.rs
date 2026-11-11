//! Les fichiers ouverts depuis Furet (ouverts, montrés, copiés, glissés) : ils remontent dans les résultats
//! par le nom, et la barre vide les propose. Gardé dans `historique.json` (dossier de configuration de
//! l'appli), sur ce PC seulement. Portable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Au-delà, les plus anciens sont oubliés.
const MAX: usize = 500;
/// Un fichier ouvert il y a un mois compte deux fois moins.
const DEMI_VIE_S: f64 = 30.0 * 86_400.0;
/// Le plus fort coup de pouce (le score d'un nom va de 0 à 1).
const BONUS_MAX: f32 = 0.3;

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Ouverture {
    nombre: u32,
    /// Secondes depuis 1970.
    derniere: u64,
}

static FICHIER: OnceLock<PathBuf> = OnceLock::new();
static HISTORIQUE: Mutex<Option<HashMap<PathBuf, Ouverture>>> = Mutex::new(None);

fn maintenant() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn avec<R>(f: impl FnOnce(&mut HashMap<PathBuf, Ouverture>) -> R) -> R {
    let mut garde = HISTORIQUE.lock().unwrap_or_else(|e| e.into_inner());
    f(garde.get_or_insert_with(HashMap::new))
}

pub fn charger(app: &AppHandle) {
    let fichier = app.path().app_config_dir().map(|d| d.join("historique.json")).unwrap_or_else(|_| "historique.json".into());
    let lu: HashMap<PathBuf, Ouverture> =
        std::fs::read_to_string(&fichier).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let _ = FICHIER.set(fichier);
    avec(|h| *h = lu);
}

/// Note qu'un fichier vient d'être ouvert (ou montré, copié, glissé), et enregistre (sur un fil à part).
pub fn noter(chemin: &Path) {
    let copie = avec(|h| {
        let o = h.entry(chemin.to_path_buf()).or_insert(Ouverture { nombre: 0, derniere: 0 });
        o.nombre = o.nombre.saturating_add(1);
        o.derniere = maintenant();
        if h.len() > MAX {
            let mut tous: Vec<(PathBuf, u64)> = h.iter().map(|(c, o)| (c.clone(), o.derniere)).collect();
            tous.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
            for (c, _) in tous.into_iter().skip(MAX) {
                h.remove(&c);
            }
        }
        h.clone()
    });
    std::thread::spawn(move || {
        let Some(fichier) = FICHIER.get() else { return };
        if let Some(d) = fichier.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(texte) = serde_json::to_string(&copie) {
            let _ = std::fs::write(fichier, texte);
        }
    });
}

/// Le coup de pouce d'un fichier souvent et récemment ouvert : de 0 à `BONUS_MAX`.
pub fn bonus(chemin: &Path) -> f32 {
    avec(|h| {
        h.get(chemin).map_or(0.0, |o| {
            let age = maintenant().saturating_sub(o.derniere) as f64;
            let frequence = (1.0 + o.nombre as f64).ln() / 3f64.ln(); // 1 pour 2 ouvertures, 2 pour 8
            let recence = 0.5f64.powf(age / DEMI_VIE_S);
            (0.12 * frequence * recence).min(BONUS_MAX as f64) as f32
        })
    })
}

/// Les fichiers ouverts le plus récemment (qui existent encore), avec le moment de leur dernière ouverture.
pub fn recents(n: usize) -> Vec<(PathBuf, u64)> {
    let mut tous: Vec<(PathBuf, u64)> = avec(|h| h.iter().map(|(c, o)| (c.clone(), o.derniere)).collect());
    tous.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
    tous.into_iter().filter(|(c, _)| c.exists()).take(n).collect()
}
