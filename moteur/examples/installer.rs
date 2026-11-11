//! Essai : installe tous les paquets dans un dossier vide, comme au premier lancement.
//! cargo run --release -p furet-moteur --example installer -- <dossier>

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use furet_moteur::installation::{self, Groupe};

fn main() -> anyhow::Result<()> {
    let racine = std::path::PathBuf::from(std::env::args().nth(1).expect("dossier"));
    let paquets = installation::manquants(&racine, &[Groupe::Essentiel, Groupe::Transcription, Groupe::Visages]);
    println!("{} paquet(s) à installer", paquets.len());
    let debut = Instant::now();
    let mut dernier = String::new();
    installation::installer(&racine, &paquets, &AtomicBool::new(false), &mut |a| {
        let ligne = format!("{} : {} Mo sur {} Mo", a.nom, a.fait >> 20, a.total >> 20);
        // Une ligne par paquet et par tranche de 100 Mo.
        let cle = format!("{} {}", a.nom, a.fait / (100 << 20));
        if cle != dernier {
            println!("{ligne}");
            dernier = cle;
        }
    })?;
    println!("installé en {:.0} s", debut.elapsed().as_secs_f64());
    let restants = installation::manquants(&racine, &[Groupe::Essentiel, Groupe::Transcription, Groupe::Visages]);
    println!("manquants après : {}", restants.len());
    Ok(())
}
