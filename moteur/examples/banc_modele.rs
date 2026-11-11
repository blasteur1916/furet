//! Essai : débit du modèle sur la carte graphique selon la taille des lots et le nombre de lots en vol.
//! Texte inventé (aucun fichier de l'utilisateur).
//! cargo run --release -p furet-moteur --example banc_modele
use std::path::Path;
use std::time::Instant;

use furet_moteur::serveur::Serveur;
use serde_json::Value;

fn passage(i: usize) -> Value {
    // Environ 1 000 caractères, comme un passage de texte ou de code.
    let mut t = format!("title: fichier{i}.txt | text: ");
    let mots = ["maison", "rapport", "fonction", "retour", "valeur", "client", "facture", "réseau", "image", "donnée",
        "calcul", "serveur", "projet", "réunion", "tableau", "liste", "erreur", "chemin", "dossier", "période"];
    let mut k = i * 7919;
    while t.len() < 1000 {
        k = k.wrapping_mul(1103515245).wrapping_add(12345);
        t.push_str(mots[(k >> 16) % mots.len()]);
        t.push(' ');
    }
    Value::String(t)
}

fn main() -> anyhow::Result<()> {
    let racine = Path::new(r"D:\furet");
    let serveur = Serveur::demarrer(racine, false, None, &std::env::temp_dir().join("furet-banc-modele.log"))?;
    println!("modèle sur {} ({:.1} s)", serveur.appareil, serveur.demarrage.as_secs_f64());
    let total = 768;
    // Mise en route (la carte prépare ses programmes).
    serveur.client().vecteurs((0..32).map(passage).collect())?;
    for (lot, en_vol) in [(8, 2), (16, 2), (32, 2), (8, 4), (16, 4), (32, 4), (64, 4)] {
        let debut = Instant::now();
        let suivant = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..en_vol {
                let client = serveur.client();
                let suivant = &suivant;
                s.spawn(move || {
                    loop {
                        let a = suivant.fetch_add(lot, std::sync::atomic::Ordering::Relaxed);
                        if a >= total {
                            break;
                        }
                        let entrees = (a..(a + lot).min(total)).map(passage).collect();
                        client.vecteurs(entrees).expect("vecteurs");
                    }
                });
            }
        });
        let s = debut.elapsed().as_secs_f64();
        println!("lots de {lot:>2}, {en_vol} en vol : {:>5.0} passages/s ({:.1} ms par passage)", total as f64 / s, s * 1000.0 / total as f64);
    }
    Ok(())
}
