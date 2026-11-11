//! Essai : temps du parcours d'un dossier et ce qu'il contient, par type (des nombres seulement : aucun
//! nom de fichier), avec ce que la règle des bibliothèques de code met de côté.
//! cargo run --release -p furet-moteur --example banc_parcours -- D:\
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

fn main() {
    let dossier = std::env::args().nth(1).expect("dossier");
    let t = Instant::now();
    let fichiers = furet_moteur::indexation::parcours_detaille(Path::new(&dossier));
    println!("{} fichiers lisibles en {:.1} s", fichiers.len(), t.elapsed().as_secs_f64());
    // Par type : (à lire, octets, en bibliothèque).
    let mut par_genre: BTreeMap<&str, (usize, u64, usize)> = BTreeMap::new();
    // Les dossiers du premier niveau, désignés par leur rang : (à lire, en bibliothèque).
    let mut par_dossier: HashMap<String, (usize, usize)> = HashMap::new();
    for (chemin, genre, taille, bibliotheque) in &fichiers {
        let e = par_genre.entry(genre).or_default();
        let premier = chemin.strip_prefix(&dossier).ok().and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().into_owned());
        let d = par_dossier.entry(premier.unwrap_or_default()).or_default();
        if *bibliotheque {
            e.2 += 1;
            d.1 += 1;
        } else {
            e.0 += 1;
            e.1 += taille;
            d.0 += 1;
        }
    }
    for (g, (n, octets, b)) in &par_genre {
        println!("{g:<9} {n:>8} à lire ({:>6} Mo)   {b:>8} en bibliothèque (par le nom)", octets >> 20);
    }
    let total: usize = par_genre.values().map(|v| v.0).sum();
    let ecartes: usize = par_genre.values().map(|v| v.2).sum();
    println!("total : {total} à lire, {ecartes} par le nom seulement");
    let mut dossiers: Vec<(usize, usize)> = par_dossier.into_values().collect();
    dossiers.sort_by_key(|n| std::cmp::Reverse(n.0 + n.1));
    println!("dossiers du premier niveau (à lire, en bibliothèque) : {:?}", &dossiers[..dossiers.len().min(14)]);
}
