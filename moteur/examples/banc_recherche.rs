//! Essai : le temps de chaque étape d'une recherche par le contenu, comme dans l'appli (index rapide chargé,
//! serveur léger sur le processeur). N'affiche que des durées et des nombres.
//! cargo run --release -p furet-moteur --example banc_recherche
use std::path::Path;
use std::time::Instant;

use furet_moteur::filtres::Filtre;
use furet_moteur::vecteurs::IndexRapide;
use furet_moteur::{base, pertinence, serveur::Serveur};

fn main() -> anyhow::Result<()> {
    let racine = Path::new(r"D:\furet");
    let donnees = racine.join("donnees");
    let b = base::ouvrir(&donnees.join("index.sqlite"))?;
    let t = Instant::now();
    let fichier = donnees.join("vecteurs.usearch");
    let index = match IndexRapide::charger(&fichier) {
        Ok(i) => i,
        Err(_) => {
            let i = IndexRapide::construire(&b)?;
            i.sauver(&b, &fichier)?;
            i
        }
    };
    println!("index rapide : {} vecteurs chargés en {:.0} ms", index.len(), t.elapsed().as_secs_f64() * 1000.0);
    let serveur = Serveur::demarrer_requetes(racine, &std::env::temp_dir().join("furet-banc.log"))?;
    let client = serveur.client();
    let filtre = Filtre::default();
    let requetes = [
        "facture", "coucher de soleil", "photo d'un chat", "config", "main", "the", "import", "error",
        "réunion de travail", "function return", "a", "personne qui parle", "test", "data",
    ];
    println!("{:<22} {:>8} {:>8} {:>9} {:>9}", "recherche", "vecteur", "voisins", "complète", "résultats");
    for tour in 0..2 {
        for q in requetes {
            let t = Instant::now();
            let v = client.vecteur_requete(q)?;
            let ms_vecteur = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let _ = index.chercher(&v, 1200)?;
            let ms_voisins = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let requete = base::Requete { vecteur: &v, texte: q, n: 40, filtre: &filtre, seuil: pertinence::SEUIL };
            let (r, _) = base::chercher(&b, &requete, Some(&index))?;
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            if tour == 1 {
                println!("{q:<22} {ms_vecteur:>6.0} ms {ms_voisins:>5.1} ms {ms:>6.0} ms {:>9}", r.len());
            }
        }
    }
    Ok(())
}
