//! L'index rapide des vecteurs : un graphe HNSW (usearch), en mémoire, avec les vecteurs en 16 bits.
//! Environ 1 ms par recherche même avec des millions de morceaux (sqlite-vec, exhaustif : 666 ms à 500 000).
//!
//! SQLite reste la référence. Des déclencheurs y notent chaque morceau ajouté ou retiré dans
//! `journal_vecteurs`, quel que soit le programme qui indexe (l'appli ou le prototype) ; l'index rapide
//! rejoue ce journal pour rester à jour. Il est sauvegardé sur le disque avec le dernier numéro du journal
//! appliqué : au lancement, on recharge le fichier et on ne rejoue que la suite.
//!
//! Relire les vecteurs des changements est lent (≈ 1 ms chacun) : `changements` se fait sans verrou, et
//! seul `appliquer`, très rapide, modifie l'index.

use crate::serveur::DIMENSION_INDEX;
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Au-delà de ce retard, il est plus rapide de tout reconstruire que de relire les vecteurs un par un.
pub const RATTRAPAGE_MAX: usize = 20_000;

pub struct IndexRapide {
    index: Index,
    /// Le dernier numéro du journal appliqué.
    seq: i64,
}

/// Des changements lus dans le journal : pour chaque morceau, son vecteur (ajouté) ou rien (retiré).
pub struct Changements {
    jusqua: i64,
    morceaux: Vec<(i64, Option<Vec<f32>>)>,
}

fn options() -> IndexOptions {
    IndexOptions {
        dimensions: DIMENSION_INDEX,
        metric: MetricKind::Cos,
        // 16 bits par composante : 2 fois moins de mémoire que 32 bits, sans perte visible sur un cosinus.
        quantization: ScalarKind::F16,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 96,
        multi: false,
    }
}

fn usearch<T>(r: std::result::Result<T, cxx::Exception>) -> Result<T> {
    r.map_err(|e| anyhow!("usearch : {e}"))
}

fn vers_f32(octets: &[u8]) -> Vec<f32> {
    octets.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Le dernier numéro du journal (0 s'il est vide).
pub fn dernier_numero(base: &Connection) -> Result<i64> {
    Ok(base.query_row("SELECT COALESCE(MAX(seq), 0) FROM journal_vecteurs", [], |r| r.get(0))?)
}

/// Changements notés après `depuis`.
pub fn en_attente(base: &Connection, depuis: i64) -> Result<usize> {
    Ok(base.query_row("SELECT COUNT(*) FROM journal_vecteurs WHERE seq > ?1", [depuis], |r| r.get::<_, i64>(0))? as usize)
}

/// Lit au plus `max` lignes du journal après `depuis`, avec les vecteurs des morceaux ajoutés.
pub fn changements(base: &Connection, depuis: i64, max: usize) -> Result<Changements> {
    let mut req = base.prepare("SELECT seq, id, op FROM journal_vecteurs WHERE seq > ?1 ORDER BY seq LIMIT ?2")?;
    // Pour chaque morceau, seul son dernier état compte (ajouté puis retiré : retiré).
    let mut dernier: HashMap<i64, bool> = HashMap::new();
    let mut jusqua = depuis;
    for l in req.query_map([depuis, max as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)? == 1)))? {
        let (seq, id, ajout) = l?;
        dernier.insert(id, ajout);
        jusqua = seq;
    }
    let mut lire = base.prepare("SELECT embedding FROM vecteurs WHERE rowid = ?1")?;
    let mut morceaux = Vec::with_capacity(dernier.len());
    for (id, ajout) in dernier {
        // Un morceau ajouté puis retiré sans que le journal le dise encore n'a plus de vecteur : retiré.
        let vecteur =
            if ajout { lire.query_row([id], |r| r.get::<_, Vec<u8>>(0)).optional()?.map(|o| vers_f32(&o)) } else { None };
        morceaux.push((id, vecteur));
    }
    Ok(Changements { jusqua, morceaux })
}

impl Changements {
    pub fn is_empty(&self) -> bool {
        self.morceaux.is_empty()
    }
}

impl IndexRapide {
    /// L'index sauvegardé (fichier `.usearch` et son numéro de journal `.seq`).
    pub fn charger(fichier: &Path) -> Result<Self> {
        let seq: i64 = std::fs::read_to_string(fichier.with_extension("seq"))?.trim().parse()?;
        let index = usearch(Index::new(&options()))?;
        usearch(index.load(&fichier.to_string_lossy()))?;
        Ok(IndexRapide { index, seq })
    }

    /// Relit tous les vecteurs de SQLite d'une traite, puis bâtit le graphe sur tous les cœurs.
    pub fn construire(base: &Connection) -> Result<Self> {
        let seq = dernier_numero(base)?;
        let tous: Vec<(u64, Vec<f32>)> = {
            let mut req = base.prepare("SELECT rowid, embedding FROM vecteurs")?;
            req.query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, vers_f32(&r.get::<_, Vec<u8>>(1)?))))?
                .collect::<rusqlite::Result<_>>()?
        };
        let fils = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
        let index = usearch(Index::new(&options()))?;
        usearch(index.reserve_capacity_and_threads(tous.len() + tous.len() / 4 + 1024, fils))?;
        let erreur = std::sync::Mutex::new(None);
        std::thread::scope(|s| {
            for part in tous.chunks(tous.len().div_ceil(fils).max(1)) {
                let (index, erreur) = (&index, &erreur);
                s.spawn(move || {
                    for (id, v) in part {
                        if let Err(e) = index.add(*id, v) {
                            *erreur.lock().unwrap_or_else(|e| e.into_inner()) = Some(anyhow!("usearch : {e}"));
                            return;
                        }
                    }
                });
            }
        });
        if let Some(e) = erreur.into_inner().unwrap_or_else(|e| e.into_inner()) {
            return Err(e);
        }
        Ok(IndexRapide { index, seq })
    }

    pub fn seq(&self) -> i64 {
        self.seq
    }

    /// Applique des changements lus avec `changements` (rapide : pas de lecture de la base).
    pub fn appliquer(&mut self, c: Changements) -> Result<()> {
        let ajouts = c.morceaux.iter().filter(|(_, v)| v.is_some()).count();
        if self.index.size() + ajouts > self.index.capacity() {
            usearch(self.index.reserve((self.index.capacity() + ajouts) * 3 / 2 + 1024))?;
        }
        for (id, vecteur) in c.morceaux {
            let cle = id as u64;
            if self.index.contains(cle) {
                usearch(self.index.remove(cle))?;
            }
            if let Some(v) = vecteur {
                usearch(self.index.add(cle, &v))?;
            }
        }
        self.seq = self.seq.max(c.jusqua);
        Ok(())
    }

    /// Les `k` morceaux les plus proches : (id du morceau, distance cosinus).
    pub fn chercher(&self, vecteur: &[f32], k: usize) -> Result<Vec<(i64, f32)>> {
        if self.index.size() == 0 {
            return Ok(Vec::new());
        }
        let trouves = usearch(self.index.search(vecteur, k))?;
        Ok(trouves.keys.iter().zip(&trouves.distances).map(|(k, d)| (*k as i64, *d)).collect())
    }

    pub fn len(&self) -> usize {
        self.index.size()
    }

    pub fn is_empty(&self) -> bool {
        self.index.size() == 0
    }

    /// Mémoire approximative de l'index, en octets.
    pub fn memoire(&self) -> usize {
        self.index.memory_usage()
    }

    /// Sauvegarde l'index (fichier temporaire puis renommage : jamais à moitié écrit), puis oublie la
    /// partie du journal qu'il contient.
    pub fn sauver(&self, base: &Connection, fichier: &Path) -> Result<()> {
        let provisoire = fichier.with_extension("usearch.tmp");
        usearch(self.index.save(&provisoire.to_string_lossy()))?;
        std::fs::rename(&provisoire, fichier).context("sauvegarde de l'index rapide")?;
        std::fs::write(fichier.with_extension("seq"), self.seq.to_string())?;
        base.execute("DELETE FROM journal_vecteurs WHERE seq <= ?1", [self.seq])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Qualité et vitesse de l'index rapide sur un vrai index : pour 200 morceaux pris comme requêtes,
    /// part des 10 vrais plus proches (recherche exhaustive) que le graphe retrouve.
    /// FURET_BASE=chemin\index.sqlite cargo test --release -p furet-moteur rappel -- --ignored --nocapture
    #[test]
    #[ignore]
    fn rappel_et_vitesse() {
        let chemin = std::env::var("FURET_BASE").expect("FURET_BASE");
        let base = crate::base::ouvrir(Path::new(&chemin)).unwrap();
        let t = Instant::now();
        let rapide = IndexRapide::construire(&base).unwrap();
        println!("construit : {} morceaux en {} ms, {} Mo", rapide.len(), t.elapsed().as_millis(), rapide.memoire() >> 20);

        let mut req = base.prepare("SELECT rowid, embedding FROM vecteurs ORDER BY random() LIMIT 200").unwrap();
        let requetes: Vec<Vec<f32>> = req.query_map([], |r| Ok(vers_f32(&r.get::<_, Vec<u8>>(1)?))).unwrap().map(|r| r.unwrap()).collect();
        let mut exact = base.prepare("SELECT rowid FROM vecteurs WHERE embedding MATCH ?1 AND k = 10").unwrap();
        let mut ms_exact = 0.0;
        let vrais: Vec<Vec<i64>> = requetes
            .iter()
            .map(|q| {
                let octets: Vec<u8> = q.iter().flat_map(|x| x.to_le_bytes()).collect();
                let t = Instant::now();
                let v = exact.query_map([octets], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
                ms_exact += t.elapsed().as_secs_f64() * 1000.0;
                v
            })
            .collect();
        let n = requetes.len() as f64;
        println!("recherche exhaustive : {:.1} ms", ms_exact / n);
        for exploration in [96, 200, 400] {
            rapide.index.change_expansion_search(exploration);
            let (mut trouves, mut total, mut ms) = (0, 0, 0.0);
            for (q, v) in requetes.iter().zip(&vrais) {
                let t = Instant::now();
                let approches: Vec<i64> = rapide.chercher(q, 10).unwrap().into_iter().map(|(id, _)| id).collect();
                ms += t.elapsed().as_secs_f64() * 1000.0;
                trouves += v.iter().filter(|x| approches.contains(x)).count();
                total += v.len();
            }
            println!(
                "exploration {exploration} : rappel à 10 {:.1} %, {:.2} ms par recherche",
                100.0 * trouves as f64 / total as f64,
                ms / n
            );
        }
    }
}
